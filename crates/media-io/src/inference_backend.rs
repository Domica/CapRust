//! Abstraction over ONNX inference backends: tract-onnx (CPU, always
//! available) and ONNX Runtime (ort) with optional CUDA (NVIDIA) or
//! DirectML (AMD) execution providers.
//!
//! The public interface returns all output tensors as flat `Vec<f32>`
//! slices in NCHW order. Single-output models (GFPGAN) get one element;
//! multi-output models (SCRFD with 9 heads) get all of them.
//! Pre/post-processing stays in the individual model modules.

use anyhow::{anyhow, Result};
use serde_json;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::{Mutex, OnceLock};
use tract_onnx::prelude::*;

#[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
use ort::environment::Environment;
#[cfg(feature = "gpu-directml")]
use ort::ep::DirectML;
#[cfg(feature = "gpu-cuda")]
use ort::ep::CUDA;
#[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
use ort::session::Session;
#[cfg(any(feature = "gpu-cuda", feature = "gpu-directml"))]
use ort::value::TensorValueType;

use crate::gpu_plugin;

/// Which backend to use for AI inference.
///
/// `ExternalCuda` / `ExternalDirectML` use a separately downloaded GPU
/// plugin binary (installed to %LOCALAPPDATA%\CapRust\gpu\). They keep
/// the standard EXE small (~27 MB) while letting users opt into GPU
/// acceleration on demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendType {
    /// Pure CPU via tract-onnx (always available, no GPU needed).
    Cpu,
    /// NVIDIA GPU via ONNX Runtime CUDA execution provider (built into EXE).
    Cuda,
    /// AMD GPU via ONNX Runtime DirectML execution provider (built into EXE).
    DirectML,
    /// NVIDIA GPU via externally downloaded plugin binary.
    ExternalCuda,
    /// AMD GPU via externally downloaded plugin binary.
    ExternalDirectML,
    /// Auto-detect: try CUDA (NVIDIA), then DirectML (AMD), fall back to CPU.
    /// Does NOT select external backends — those require manual install.
    Auto,
}

impl BackendType {
    /// Auto-detect the best available GPU backend.
    /// Tries CUDA (NVIDIA), then DirectML (AMD), falls back to CPU.
    /// Result is cached after first detection.
    ///
    /// Does NOT select `ExternalCuda` / `ExternalDirectML` — those
    /// require the user to manually install the plugin first.
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

/// Returns `true` if this EXE was compiled with CUDA support
/// (`--features gpu-cuda`). When true, CUDA is already built in and no
/// external plugin download is needed.
pub fn has_builtin_cuda() -> bool {
    #[cfg(feature = "gpu-cuda")]
    {
        true
    }
    #[cfg(not(feature = "gpu-cuda"))]
    {
        false
    }
}

/// Returns `true` if this EXE was compiled with DirectML support
/// (`--features gpu-directml`). When true, DirectML is already built in
/// and no external plugin download is needed.
pub fn has_builtin_directml() -> bool {
    #[cfg(feature = "gpu-directml")]
    {
        true
    }
    #[cfg(not(feature = "gpu-directml"))]
    {
        false
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

/// Wraps an external GPU plugin binary as an `InferenceBackend`.
///
/// The binary lives in %LOCALAPPDATA%\CapRust\gpu\ (installed by
/// `gpu_plugin::install`) and speaks a line-based protocol over
/// stdin/stdout:
///   `LOAD <model_path> <shape_json>`   -> `OK` | `ERR <msg>`
///   `RUN <input_path> <output_path>`   -> `OK` | `ERR <msg>`
///   `QUIT`                              -> process exits
///
/// The model is loaded once when the backend is created. Each `run()`
/// writes the input tensor to a temp file, sends `RUN`, and reads the
/// output tensor from a temp file.
///
/// Compiled without any GPU feature — it only shells out to an external
/// process, so it works even in the standard CPU-only build.
pub struct ExternalBackend {
    inner: Mutex<ExternalBackendInner>,
}

struct ExternalBackendInner {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    #[allow(dead_code)]
    model_path: PathBuf,
    shape: [i64; 4],
}

impl ExternalBackend {
    /// Launch the plugin binary and load the model into it.
    pub fn load(
        binary_path: &Path,
        model_path: &Path,
        shape: [i64; 4],
        label: &str,
    ) -> Result<Self> {
        let mut cmd = gpu_plugin::plugin_command(binary_path, model_path, shape);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("failed to spawn GPU plugin binary: {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to capture plugin stdin"))?;
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| anyhow!("failed to capture plugin stdout"))?,
        );

        let mut inner = ExternalBackendInner {
            child,
            stdin,
            stdout,
            model_path: model_path.to_path_buf(),
            shape,
        };

        // Send LOAD command and wait for OK.
        let shape_json = serde_json::to_string(&shape)?;
        inner.send_command(&format!("LOAD {} {}\n", model_path.display(), shape_json))?;
        let response = inner.read_response()?;
        if response != "OK" {
            return Err(anyhow!(
                "GPU plugin failed to load {label} model: {response}"
            ));
        }

        tracing::info!(
            "external GPU plugin: loaded {} on {}",
            model_path.display(),
            binary_path.display()
        );

        Ok(Self {
            inner: Mutex::new(inner),
        })
    }
}

impl InferenceBackend for ExternalBackend {
    fn run(&self, input: &[f32], shape: [i64; 4]) -> Result<Vec<Vec<f32>>> {
        if shape != self.inner.lock().unwrap().shape {
            return Err(anyhow!(
                "external GPU backend: input shape {:?} != configured shape {:?}",
                shape,
                self.inner.lock().unwrap().shape
            ));
        }

        let mut inner = self.inner.lock().unwrap();

        // Write input tensor to a temp file.
        let input_temp = std::env::temp_dir().join(format!(
            "caprust-gpu-input-{}-{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let input_path = input_temp.as_path();
        {
            let mut file = std::fs::File::create(input_path)
                .map_err(|e| anyhow!("failed to open input temp file: {e}"))?;
            let bytes: Vec<u8> = input.iter().flat_map(|&f| f.to_le_bytes()).collect();
            file.write_all(&bytes)
                .map_err(|e| anyhow!("failed to write input temp file: {e}"))?;
        }

        let output_temp = std::env::temp_dir().join(format!(
            "caprust-gpu-output-{}-{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let output_path = output_temp.as_path();

        // Send RUN command.
        inner.send_command(&format!(
            "RUN {} {}\n",
            input_path.display(),
            output_path.display()
        ))?;
        let response = inner.read_response()?;
        if response != "OK" {
            return Err(anyhow!("external GPU inference failed: {response}"));
        }

        // Read output tensor from temp file.
        let output_data = std::fs::read(output_path)
            .map_err(|e| anyhow!("failed to read output temp file: {e}"))?;
        if output_data.len() % 4 != 0 {
            return Err(anyhow!(
                "external GPU plugin returned invalid output size: {} bytes",
                output_data.len()
            ));
        }
        let output_f32: Vec<f32> = output_data
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect();

        Ok(vec![output_f32])
    }
}

impl Drop for ExternalBackend {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.inner.lock() {
            let _ = inner.stdin.write_all(b"QUIT\n");
            let _ = inner.stdin.flush();
            let _ = inner.child.wait();
        }
    }
}

impl ExternalBackendInner {
    fn send_command(&mut self, cmd: &str) -> Result<()> {
        self.stdin
            .write_all(cmd.as_bytes())
            .map_err(|e| anyhow!("failed to send command to GPU plugin: {e}"))?;
        self.stdin
            .flush()
            .map_err(|e| anyhow!("failed to flush command to GPU plugin: {e}"))?;
        Ok(())
    }

    fn read_response(&mut self) -> Result<String> {
        let mut response = String::new();
        self.stdout
            .read_line(&mut response)
            .map_err(|e| anyhow!("failed to read response from GPU plugin: {e}"))?;
        Ok(response.trim().to_string())
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
    #[allow(dead_code)]
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
            BackendType::ExternalCuda | BackendType::ExternalDirectML => {
                return Err(anyhow!("OrtBackend does not support external backends"));
            }
            BackendType::Auto => {
                unreachable!(
                    "Auto is resolved to a concrete backend before OrtBackend::load is called"
                )
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

        let mut session = self
            .session
            .lock()
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
    /// The backend type that was actually selected (after Auto resolution
    /// and any CPU fallback). Exposed so callers can report diagnostics.
    backend_type: BackendType,
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
            BackendType::ExternalCuda | BackendType::ExternalDirectML => {
                let binary_path = match backend {
                    BackendType::ExternalCuda => gpu_plugin::cuda_binary_path(),
                    BackendType::ExternalDirectML => gpu_plugin::directml_binary_path(),
                    _ => unreachable!(),
                };
                match binary_path {
                    Some(bin) => match ExternalBackend::load(&bin, path, shape, label) {
                        Ok(b) => Box::new(b),
                        Err(e) => {
                            tracing::warn!(
                                "External GPU backend {:?} failed to load {}, falling back to CPU: {e}",
                                backend,
                                path.display()
                            );
                            Box::new(TractBackend::load(path, shape, label)?)
                        }
                    },
                    None => {
                        tracing::warn!(
                            "External GPU backend {:?} requested but plugin not installed, falling back to CPU",
                            backend
                        );
                        Box::new(TractBackend::load(path, shape, label)?)
                    }
                }
            }
            BackendType::Auto => {
                unreachable!("Auto is resolved to a concrete backend before this match")
            }
        };

        Ok(Self {
            backend: backend_obj,
            input_shape: shape,
            backend_type: backend,
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

    /// Returns the backend type actually in use (after Auto resolution
    /// and any CPU fallback).
    pub fn backend_type(&self) -> BackendType {
        self.backend_type
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
