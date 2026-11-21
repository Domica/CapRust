//! Abstraction over ONNX inference backends: tract-onnx (CPU, always
//! available) and ONNX Runtime (ort) with optional CUDA (NVIDIA) or
//! DirectML (AMD) execution providers.
//!
//! The public interface returns all output tensors as flat `Vec<f32>`
//! slices in NCHW order. Single-output models (GFPGAN) get one element;
//! multi-output models (SCRFD with 9 heads) get all of them.
//! Pre/post-processing stays in the individual model modules.

use anyhow::{anyhow, Result};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use tract_onnx::prelude::*;

#[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
use ort::environment::Environment;
#[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
use ort::session::Session;
#[cfg(feature = "gpu-cuda")]
use ort::ep::CUDA;
#[cfg(feature = "gpu-directml")]
use ort::ep::DirectML;
#[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
use ort::value::TensorValueType;

/// Which backend to use for AI inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendType {
    /// Pure CPU via tract-onnx (always available, no GPU needed).
    Cpu,
    /// NVIDIA GPU via ONNX Runtime CUDA execution provider.
    Cuda,
    /// AMD GPU via ONNX Runtime DirectML execution provider.
    DirectML,
    /// Auto-detect: try CUDA (NVIDIA), then DirectML (AMD), fall back to CPU.
    Auto,
}

impl BackendType {
    /// Auto-detect the best available GPU backend.
    /// Tries CUDA (NVIDIA), then DirectML (AMD), falls back to CPU.
    /// Result is cached after first detection.
    pub fn auto_detect() -> Self {
        static DETECTED: OnceLock<BackendType> = OnceLock::new();
        *DETECTED.get_or_init(|| {
            #[cfg(feature = "gpu-cuda")]
            {
                if let Ok(env) = ort_env() {
                    if let Ok(builder) = Session::builder(env) {
                        if builder
                            .with_execution_providers([CUDA::default().build()])
                            .is_ok()
                        {
                            return BackendType::Cuda;
                        }
                    }
                }
            }
            #[cfg(feature = "gpu-directml")]
            {
                if let Ok(env) = ort_env() {
                    if let Ok(builder) = Session::builder(env) {
                        if builder
                            .with_execution_providers([DirectML::default().build()])
                            .is_ok()
                        {
                            return BackendType::DirectML;
                        }
                    }
                }
            }
            BackendType::Cpu
        })
    }
}

/// Returns a shared ONNX Runtime environment. Only one environment is
/// allowed per process, so we cache it in a `OnceLock`.
#[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
fn ort_env() -> Result<&'static Environment> {
    static ENV: OnceLock<Result<Environment, ort::Error>> = OnceLock::new();
    ENV.get_or_init(|| ort::init().with_name("caprust").build())
        .as_ref()
        .map_err(|e| anyhow!("ort environment: {e:?}"))
}

/// Trait implemented by both tract and ort backends.
///
/// `input` is a flat `f32` slice in NCHW order matching `shape`
/// (e.g. `[1, 3, 512, 512]`). Returns all output tensors flattened to
/// `Vec<f32>`, one inner Vec per output tensor.
pub trait InferenceBackend: Send + Sync {
    fn run(&self, input: &[f32], shape: [i64; 4]) -> Result<Vec<Vec<f32>>>;

    /// Convenience: return only the first output as a flat `Vec<f32>`.
    fn run_single(&self, input: &[f32], shape: [i64; 4]) -> Result<Vec<f32>> {
        let outputs = self.run(input, shape)?;
        outputs
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("model produced no outputs"))
    }
}

/// Wraps a tract-onnx TypedRunnableModel as an `InferenceBackend`.
pub struct TractBackend {
    model: std::sync::Arc<
        tract_onnx::prelude::RunnableModel<
            tract_onnx::prelude::TypedFact,
            Box<dyn tract_onnx::prelude::TypedOp>,
        >,
    >,
}

impl TractBackend {
    pub fn load(path: &Path, shape: [i64; 4], label: &str) -> Result<Self> {
        use crate::onnx_common::load_typed_pinned;
        let model = load_typed_pinned(path, label, shape)?;
        Ok(Self { model })
    }
}

impl InferenceBackend for TractBackend {
    fn run(&self, input: &[f32], shape: [i64; 4]) -> Result<Vec<Vec<f32>>> {
        let arr = tract_ndarray::Array4::<f32>::from_shape_vec(
            (
                shape[0] as usize,
                shape[1] as usize,
                shape[2] as usize,
                shape[3] as usize,
            ),
            input.to_vec(),
        )
        .map_err(|e| anyhow!("reshape tract input: {e:?}"))?;

        let input_tensor: Tensor = arr.into();
        let outputs = self
            .model
            .run(tvec!(input_tensor.into()))
            .map_err(|e| anyhow!("tract inference: {e:?}"))?;

        let mut result = Vec::with_capacity(outputs.len());
        for output in outputs.into_iter() {
            let arr_view = output
                .into_tensor()
                .into_plain_array::<f32>()
                .map_err(|e| anyhow!("convert tract output: {e:?}"))?;
            result.push(arr_view.into_raw_vec_and_offset().0);
        }
        Ok(result)
    }
}

/// Wraps an ONNX Runtime Session as an `InferenceBackend`.
///
/// Compiled only when `gpu-cuda` or `gpu-directml` feature is enabled.
/// The `Session` is wrapped in a `Mutex` because `Session::run` takes
/// `&mut self`.
#[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
pub struct OrtBackend {
    session: Mutex<Session>,
    input_name: String,
    output_count: usize,
}

#[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
impl OrtBackend {
    pub fn load(path: &Path, shape: [i64; 4], label: &str, backend: BackendType) -> Result<Self> {
        let _ = shape;
        let _ = label;
        let env = ort_env()?;

        let mut builder =
            Session::builder(env).map_err(|e| anyhow!("ort session builder: {e:?}"))?;

        match backend {
            BackendType::Cuda => {
                #[cfg(feature = "gpu-cuda")]
                {
                    builder = builder
                        .with_execution_providers([CUDA::default().build()])
                        .map_err(|e| anyhow!("ort CUDA provider: {e:?}"))?;
                }
                #[cfg(not(feature = "gpu-cuda"))]
                return Err(anyhow!(
                    "CUDA backend requested but gpu-cuda feature not compiled in"
                ));
            }
            BackendType::DirectML => {
                #[cfg(feature = "gpu-directml")]
                {
                    builder = builder
                        .with_execution_providers([DirectML::default().build()])
                        .map_err(|e| anyhow!("ort DirectML provider: {e:?}"))?;
                }
                #[cfg(not(feature = "gpu-directml"))]
                return Err(anyhow!(
                    "DirectML backend requested but gpu-directml feature not compiled in"
                ));
            }
            BackendType::Cpu => {
                // CPU is the default; no provider needed.
            }
            BackendType::Auto => {
                unreachable!("Auto is resolved to a concrete backend before OrtBackend::load is called")
            }
        }

        let session = builder
            .commit_from_file(path)
            .map_err(|e| anyhow!("ort load model {}: {e:?}", path.display()))?;

        let output_count = session.outputs().len();

        let input_name = session
            .inputs()
            .iter()
            .next()
            .ok_or_else(|| anyhow!("ort model {} has no inputs", path.display()))?
            .name()
            .to_string();

        tracing::info!(
            "ort: loaded {} on {:?} backend, {} output(s)",
            path.display(),
            backend,
            output_count
        );

        Ok(Self {
            session: Mutex::new(session),
            input_name,
            output_count,
        })
    }
}

#[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
impl InferenceBackend for OrtBackend {
    fn run(&self, input: &[f32], shape: [i64; 4]) -> Result<Vec<Vec<f32>>> {
        let arr = tract_ndarray::Array4::<f32>::from_shape_vec(
            (
                shape[0] as usize,
                shape[1] as usize,
                shape[2] as usize,
                shape[3] as usize,
            ),
            input.to_vec(),
        )
        .map_err(|e| anyhow!("reshape ort input: {e:?}"))?;

        let input_tensor = ort::value::Tensor::from_array(arr)
            .map_err(|e| anyhow!("ort create input tensor: {e:?}"))?;

        let mut session = self.session.lock()
            .map_err(|e| anyhow!("ort session lock poisoned: {e}"))?;
        let outputs = session
            .run(ort::inputs![self.input_name.clone() => input_tensor])
            .map_err(|e| anyhow!("ort inference: {e:?}"))?;

        let mut result = Vec::with_capacity(outputs.len());
        for (_, value) in outputs.into_iter() {
            let tensor: ort::value::Tensor<f32> = value
                .downcast::<TensorValueType<f32>>()
                .map_err(|e| anyhow!("ort downcast output: {e:?}"))?;
            let array = tensor
                .try_extract_array::<f32>()
                .map_err(|e| anyhow!("ort extract output: {e:?}"))?;
            result.push(array.into_owned().into_raw_vec_and_offset().0);
        }
        Ok(result)
    }
}

/// Unified inference session. Holds either a tract or ort backend.
pub struct InferenceSession {
    backend: Box<dyn InferenceBackend>,
    input_shape: [i64; 4],
}

impl InferenceSession {
    /// Load a model from `path` with the given input `shape` using the
    /// selected `backend`. Falls back to CPU tract if GPU backend fails
    /// to load (e.g. no CUDA-capable GPU or driver missing).
    pub fn load(path: &Path, shape: [i64; 4], label: &str, backend: BackendType) -> Result<Self> {
        let backend = match backend {
            BackendType::Auto => BackendType::auto_detect(),
            other => other,
        };
        let backend_obj: Box<dyn InferenceBackend> = match backend {
            BackendType::Cpu => Box::new(TractBackend::load(path, shape, label)?),
            BackendType::Cuda | BackendType::DirectML => {
                #[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
                {
                    match OrtBackend::load(path, shape, label, backend) {
                        Ok(b) => Box::new(b),
                        Err(e) => {
                            tracing::warn!(
                                "GPU backend {:?} failed to load {}, falling back to CPU: {e}",
                                backend,
                                path.display()
                            );
                            Box::new(TractBackend::load(path, shape, label)?)
                        }
                    }
                }
                #[cfg(not(any(feature = "gpu-cuda", feature = "gpu-directml")))]
                {
                    let _ = shape;
                    tracing::warn!(
                        "GPU backend {:?} requested but ort features not compiled, falling back to CPU",
                        backend
                    );
                    Box::new(TractBackend::load(path, shape, label)?)
                }
            }
            BackendType::Auto => {
                unreachable!("Auto is resolved to a concrete backend before this match")
            }
        };

        Ok(Self {
            backend: backend_obj,
            input_shape: shape,
        })
    }

    /// Run inference on a flat NCHW `f32` input. Returns all outputs.
    pub fn run(&self, input: &[f32]) -> Result<Vec<Vec<f32>>> {
        self.backend.run(input, self.input_shape)
    }

    /// Run inference on an `Array4<f32>` (e.g. from preprocessing).
    /// Returns all outputs as flat `Vec<f32>` slices.
    pub fn run_array(&self, input: &tract_ndarray::Array4<f32>) -> Result<Vec<Vec<f32>>> {
        let shape: [i64; 4] = [
            input.shape()[0] as i64,
            input.shape()[1] as i64,
            input.shape()[2] as i64,
            input.shape()[3] as i64,
        ];
        let flat: Vec<f32> = input.iter().copied().collect();
        self.backend.run(&flat, shape)
    }

    /// Run inference on an `Array4<f32>`, return only the first output.
    pub fn run_array_single(&self, input: &tract_ndarray::Array4<f32>) -> Result<Vec<f32>> {
        let outputs = self.run_array(input)?;
        outputs
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("model produced no outputs"))
    }

    pub fn shape(&self) -> [i64; 4] {
        self.input_shape
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_type_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<BackendType>();
    }

    #[test]
    fn cpu_backend_always_available() {
        let _ = BackendType::Cpu;
        let _ = BackendType::Cuda;
        let _ = BackendType::DirectML;
    }
}
