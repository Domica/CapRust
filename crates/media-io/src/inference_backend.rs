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
use tract_onnx::prelude::*;

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
        static DETECTED: std::sync::OnceLock<BackendType> = std::sync::OnceLock::new();
        *DETECTED.get_or_init(|| {
            #[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
            {
                // Try CUDA first (NVIDIA)
                if let Ok(env) = ort::Environment::builder()
                    .with_name("caprust-detect-cuda")
                    .build()
                {
                    if let Ok(mut builder) = ort::SessionBuilder::new(&env) {
                        if builder.with_cuda_provider().is_ok() {
                            return BackendType::Cuda;
                        }
                    }
                }
                // Try DirectML (AMD)
                if let Ok(env) = ort::Environment::builder()
                    .with_name("caprust-detect-directml")
                    .build()
                {
                    if let Ok(mut builder) = ort::SessionBuilder::new(&env) {
                        if builder.with_directml_provider().is_ok() {
                            return BackendType::DirectML;
                        }
                    }
                }
            }
            BackendType::Cpu
        })
    }
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
/// Holds both `Environment` and `Session` because `Tensor::from_array`
/// needs an `&Environment`.
#[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
pub struct OrtBackend {
    env: ort::Environment,
    session: ort::Session,
    input_name: String,
    output_count: usize,
}

#[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
impl OrtBackend {
    pub fn load(path: &Path, shape: [i64; 4], label: &str, backend: BackendType) -> Result<Self> {
        let env = ort::Environment::builder()
            .with_name(label)
            .build()
            .map_err(|e| anyhow!("ort environment: {e:?}"))?;

        let mut builder =
            ort::SessionBuilder::new(&env).map_err(|e| anyhow!("ort session builder: {e:?}"))?;

        match backend {
            BackendType::Cuda => {
                builder = builder
                    .with_cuda_provider()
                    .map_err(|e| anyhow!("ort CUDA provider: {e:?}"))?;
            }
            BackendType::DirectML => {
                builder = builder
                    .with_directml_provider()
                    .map_err(|e| anyhow!("ort DirectML provider: {e:?}"))?;
            }
            BackendType::Cpu => {
                // CPU is the default; no provider needed.
            }
        }

        let session = builder
            .with_model_file(path)
            .map_err(|e| anyhow!("ort load model {}: {e:?}", path.display()))?;

        let output_count = session.output_names.len();

        tracing::info!(
            "ort: loaded {} on {:?} backend, {} output(s)",
            path.display(),
            backend,
            output_count
        );

        Ok(Self {
            env,
            session,
            input_name: session
                .input_names
                .iter()
                .next()
                .ok_or_else(|| anyhow!("ort model {} has no inputs", path.display()))?
                .to_string(),
            output_count,
        })
    }
}

#[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
impl InferenceBackend for OrtBackend {
    fn run(&self, input: &[f32], shape: [i64; 4]) -> Result<Vec<Vec<f32>>> {
        let dims: Vec<i64> = shape.to_vec();

        let input_tensor = ort::value::Tensor::from_array(&self.env, input, &dims)
            .map_err(|e| anyhow!("ort create input tensor: {e:?}"))?;

        let outputs = self
            .session
            .run(ort::inputs![self.input_name => input_tensor])
            .map_err(|e| anyhow!("ort inference: {e:?}"))?;

        let mut result = Vec::with_capacity(outputs.len());
        for output in outputs.iter() {
            let array = output
                .extract_array::<f32>()
                .map_err(|e| anyhow!("ort extract output: {e:?}"))?;
            result.push(array.into_owned().into_raw_vec());
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
