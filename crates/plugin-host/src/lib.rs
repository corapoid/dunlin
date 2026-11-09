//! Sandboxed WASM plugin host.
//!
//! The host reads plugin manifests at startup but never compiles or instantiates
//! a WASM module until the user explicitly loads one. Modules get no WASI, no
//! network, no filesystem and no host imports. Every call is bounded by fuel,
//! linear-memory and output limits.

use dunlin_knowledge::{KnowledgeObject, KnowledgeSchema};
use dunlin_plugin::{Manifest, WasmModule};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use wasmi::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

pub const MAX_WASM_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub const MAX_FAILURES: u32 = 3;

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("plugin manifest: {0}")]
    Manifest(String),
    #[error("plugin directory: {0}")]
    Io(String),
    #[error("plugin has no WASM module")]
    NoWasm,
    #[error("WASM SHA-256 mismatch")]
    HashMismatch,
    #[error("WASM module is larger than {} MiB", MAX_WASM_BYTES / 1024 / 1024)]
    WasmTooLarge,
    #[error("WASM: {0}")]
    Wasm(String),
    #[error("plugin request is larger than {} KiB", MAX_REQUEST_BYTES / 1024)]
    RequestTooLarge,
    #[error("plugin output is larger than the configured limit")]
    OutputTooLarge,
    #[error("plugin returned invalid JSON: {0}")]
    Json(String),
    #[error("plugin rejected the request: {0}")]
    Rejected(String),
    #[error("plugin disabled after repeated failures")]
    Disabled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginRequest {
    #[serde(default = "default_api")]
    pub api: u32,
    pub op: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub schema_version: Option<u32>,
    #[serde(default)]
    pub input: Value,
}

fn default_api() -> u32 {
    dunlin_knowledge::API_VERSION
}

impl PluginRequest {
    pub fn new(op: impl Into<String>) -> Self {
        Self {
            api: dunlin_knowledge::API_VERSION,
            op: op.into(),
            kind: None,
            schema_version: None,
            input: Value::Null,
        }
    }

    pub fn kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = Some(kind.into());
        self
    }

    pub fn schema_version(mut self, version: u32) -> Self {
        self.schema_version = Some(version);
        self
    }

    pub fn input(mut self, input: Value) -> Self {
        self.input = input;
        self
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PluginResponse {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub output: Value,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

impl PluginResponse {
    pub fn into_output(self) -> Result<Value, PluginError> {
        if self.ok {
            Ok(self.output)
        } else {
            Err(PluginError::Rejected(
                self.error.unwrap_or_else(|| "no error detail".into()),
            ))
        }
    }
}

#[derive(Clone, Debug)]
pub struct InstalledPlugin {
    pub manifest: Manifest,
    pub manifest_hash: String,
    pub directory: PathBuf,
    pub wasm_path: Option<PathBuf>,
}

impl InstalledPlugin {
    pub fn load(&self) -> Result<WasmPlugin, PluginError> {
        WasmPlugin::load(self)
    }
}

#[derive(Clone, Debug, Default)]
pub struct PluginRegistry {
    pub plugins: Vec<InstalledPlugin>,
    pub warnings: Vec<String>,
}

impl PluginRegistry {
    /// Read manifests only. No WASM is compiled or instantiated here.
    pub fn discover(root: &Path) -> Self {
        let mut registry = Self::default();
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) => {
                if root.exists() {
                    registry
                        .warnings
                        .push(format!("Cannot read plugin directory: {error}"));
                }
                return registry;
            }
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
        paths.sort();
        let mut ids = std::collections::HashSet::new();
        for path in paths {
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                continue;
            }
            if !path.join(dunlin_plugin::MANIFEST).is_file() {
                continue;
            }
            match dunlin_plugin::load_dir_with_hash(&path) {
                Ok((manifest, manifest_hash)) => {
                    if !ids.insert(manifest.id.clone()) {
                        registry
                            .warnings
                            .push(format!("Duplicate plugin id {}", manifest.id));
                        continue;
                    }
                    let wasm_path = manifest.wasm.as_ref().map(|wasm| path.join(&wasm.path));
                    if let Some(wasm_path) = &wasm_path {
                        match std::fs::symlink_metadata(wasm_path) {
                            Ok(metadata)
                                if metadata.is_file() && !metadata.file_type().is_symlink() => {}
                            Ok(_) => registry
                                .warnings
                                .push(format!("{}: WASM path is not a regular file", manifest.id)),
                            Err(error) => registry
                                .warnings
                                .push(format!("{}: WASM file missing: {error}", manifest.id)),
                        }
                    }
                    registry.plugins.push(InstalledPlugin {
                        manifest,
                        manifest_hash,
                        directory: path,
                        wasm_path,
                    });
                }
                Err(error) => registry
                    .warnings
                    .push(format!("{}: {error}", path.display())),
            }
        }
        registry
    }

    pub fn get(&self, id: &str) -> Option<&InstalledPlugin> {
        self.plugins.iter().find(|plugin| plugin.manifest.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut InstalledPlugin> {
        self.plugins
            .iter_mut()
            .find(|plugin| plugin.manifest.id == id)
    }
}

struct HostState {
    limits: StoreLimits,
}

pub struct WasmPlugin {
    manifest: Manifest,
    module: Module,
    engine: Engine,
    failures: AtomicU32,
    disabled: AtomicBool,
}

impl WasmPlugin {
    pub fn load(plugin: &InstalledPlugin) -> Result<Self, PluginError> {
        let wasm = plugin.manifest.wasm.as_ref().ok_or(PluginError::NoWasm)?;
        let path = plugin
            .wasm_path
            .as_ref()
            .ok_or_else(|| PluginError::Io("WASM path is unavailable".into()))?;
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| PluginError::Io(format!("{}: {error}", path.display())))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(PluginError::Io("WASM path is not a regular file".into()));
        }
        if metadata.len() > MAX_WASM_BYTES {
            return Err(PluginError::WasmTooLarge);
        }
        let bytes = std::fs::read(path)
            .map_err(|error| PluginError::Io(format!("{}: {error}", path.display())))?;
        Self::from_bytes(plugin.manifest.clone(), wasm.clone(), &bytes)
    }

    pub fn from_bytes(
        manifest: Manifest,
        wasm: WasmModule,
        bytes: &[u8],
    ) -> Result<Self, PluginError> {
        if bytes.len() as u64 > MAX_WASM_BYTES {
            return Err(PluginError::WasmTooLarge);
        }
        let hash = hex::encode(Sha256::digest(bytes));
        if !hash.eq_ignore_ascii_case(&wasm.sha256) {
            return Err(PluginError::HashMismatch);
        }
        let mut config = Config::default();
        config.consume_fuel(true);
        // A Rust-compiled plugin can have a deep serde call chain. Bound the
        // interpreter recursion so a malicious module traps instead of
        // exhausting the host thread stack.
        config.set_max_recursion_depth(16);
        config.set_min_stack_height(64 * 1024);
        config.set_max_stack_height(8 * 1024 * 1024);
        let engine = Engine::new(&config);
        let module =
            Module::new(&engine, bytes).map_err(|error| PluginError::Wasm(error.to_string()))?;
        Ok(Self {
            manifest,
            module,
            engine,
            failures: AtomicU32::new(0),
            disabled: AtomicBool::new(false),
        })
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled.load(Ordering::SeqCst)
    }

    pub fn failures(&self) -> u32 {
        self.failures.load(Ordering::SeqCst)
    }

    pub fn reset_failures(&self) {
        self.failures.store(0, Ordering::SeqCst);
        self.disabled.store(false, Ordering::SeqCst);
    }

    pub fn describe(&self) -> Result<KnowledgeSchema, PluginError> {
        if let Some(schema) = &self.manifest.knowledge {
            return Ok(schema.clone());
        }
        let value = self.call(PluginRequest::new("describe"))?.into_output()?;
        if let Some(text) = value.get("schema_text").and_then(Value::as_str) {
            return serde_json::from_str(text)
                .map_err(|error| PluginError::Json(error.to_string()));
        }
        serde_json::from_value(value).map_err(|error| PluginError::Json(error.to_string()))
    }

    pub fn validate(
        &self,
        kind: &str,
        object: &KnowledgeObject,
    ) -> Result<Vec<String>, PluginError> {
        let request = PluginRequest::new("validate")
            .kind(kind)
            .schema_version(object.schema_version)
            .input(
                serde_json::to_value(object)
                    .map_err(|error| PluginError::Json(error.to_string()))?,
            );
        let value = self.call(request)?.into_output()?;
        if let Some(errors) = value.get("errors").and_then(Value::as_array) {
            return Ok(errors
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect());
        }
        Ok(Vec::new())
    }

    pub fn context_section(
        &self,
        kind: &str,
        object: &KnowledgeObject,
    ) -> Result<String, PluginError> {
        let request = PluginRequest::new("context_section")
            .kind(kind)
            .schema_version(object.schema_version)
            .input(
                serde_json::to_value(object)
                    .map_err(|error| PluginError::Json(error.to_string()))?,
            );
        let value = self.call(request)?.into_output()?;
        Ok(value
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    pub fn call(&self, request: PluginRequest) -> Result<PluginResponse, PluginError> {
        if self.is_disabled() {
            return Err(PluginError::Disabled);
        }
        // Wasmi and Rust-compiled plugins can use a deep native call chain.
        // Run each call on a dedicated large-stack thread so a malicious or
        // pathological module traps inside the sandbox instead of aborting the
        // application process.
        let result = std::thread::scope(|scope| {
            let handle = std::thread::Builder::new()
                .name("dunlin-wasm-plugin".into())
                .stack_size(32 * 1024 * 1024)
                .spawn_scoped(scope, || self.call_inner(request))
                .map_err(|error| {
                    PluginError::Wasm(format!("could not start plugin worker: {error}"))
                })?;
            handle
                .join()
                .map_err(|_| PluginError::Wasm("plugin call panicked".into()))?
        });
        match result {
            Ok(response) => Ok(response),
            Err(error) => {
                self.record_failure();
                Err(error)
            }
        }
    }

    fn call_inner(&self, request: PluginRequest) -> Result<PluginResponse, PluginError> {
        let wasm = self.manifest.wasm.as_ref().ok_or(PluginError::NoWasm)?;
        let request_bytes =
            serde_json::to_vec(&request).map_err(|error| PluginError::Json(error.to_string()))?;
        if request_bytes.len() > MAX_REQUEST_BYTES {
            return Err(PluginError::RequestTooLarge);
        }
        if request_bytes.len() > i32::MAX as usize {
            return Err(PluginError::RequestTooLarge);
        }

        let limits = StoreLimitsBuilder::new()
            .memory_size(wasm.max_memory_bytes)
            .instances(1)
            .memories(1)
            .tables(1)
            .build();
        let mut store = Store::new(&self.engine, HostState { limits });
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(wasm.fuel)
            .map_err(|error| PluginError::Wasm(error.to_string()))?;

        let linker = Linker::<HostState>::new(&self.engine);
        let instance = linker
            .instantiate_and_start(&mut store, &self.module)
            .map_err(|error| PluginError::Wasm(error.to_string()))?;
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| PluginError::Wasm("module does not export memory".into()))?;
        let alloc: TypedFunc<i32, i32> = instance
            .get_typed_func(&store, "dunlin_alloc")
            .map_err(|error| PluginError::Wasm(error.to_string()))?;
        let free: TypedFunc<(i32, i32), ()> = instance
            .get_typed_func(&store, "dunlin_free")
            .map_err(|error| PluginError::Wasm(error.to_string()))?;
        let call: TypedFunc<(i32, i32), i64> = instance
            .get_typed_func(&store, "dunlin_call")
            .map_err(|error| PluginError::Wasm(error.to_string()))?;

        let request_len = request_bytes.len() as i32;
        let request_ptr = alloc
            .call(&mut store, request_len)
            .map_err(|error| PluginError::Wasm(error.to_string()))?;
        if request_ptr < 0 {
            return Err(PluginError::Wasm(
                "allocator returned a negative pointer".into(),
            ));
        }
        memory
            .write(&mut store, request_ptr as usize, &request_bytes)
            .map_err(|error| PluginError::Wasm(error.to_string()))?;
        let packed = call
            .call(&mut store, (request_ptr, request_len))
            .map_err(|error| PluginError::Wasm(error.to_string()))?;
        let _ = free.call(&mut store, (request_ptr, request_len));
        let response_ptr = (packed as u64 >> 32) as u32;
        let response_len = packed as u32;
        if response_len as usize > wasm.max_output_bytes {
            return Err(PluginError::OutputTooLarge);
        }
        let mut response_bytes = vec![0u8; response_len as usize];
        memory
            .read(&store, response_ptr as usize, &mut response_bytes)
            .map_err(|error| PluginError::Wasm(error.to_string()))?;
        let _ = free.call(&mut store, (response_ptr as i32, response_len as i32));
        serde_json::from_slice(&response_bytes)
            .map_err(|error| PluginError::Json(error.to_string()))
    }

    fn record_failure(&self) {
        let failures = self.failures.fetch_add(1, Ordering::SeqCst) + 1;
        if failures >= MAX_FAILURES {
            self.disabled.store(true, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wat_manifest(wasm: &str) -> Manifest {
        let hash = hex::encode(Sha256::digest(wasm.as_bytes()));
        dunlin_plugin::parse(&format!(
            r#"{{
                "id":"test","name":"Test","version":"1.0.0","api":2,
                "wasm":{{
                    "path":"plugin.wasm",
                    "sha256":"{hash}",
                    "exports":["dunlin_alloc","dunlin_free","dunlin_call"]
                }}
            }}"#
        ))
        .unwrap()
    }

    fn echo_wat() -> &'static str {
        r#"
        (module
            (memory (export "memory") 1)
            (data (i32.const 1024) "{\"ok\":true,\"output\":{\"hello\":\"world\"}}")
            (func (export "dunlin_alloc") (param i32) (result i32) (i32.const 0))
            (func (export "dunlin_free") (param i32 i32))
            (func (export "dunlin_call") (param i32 i32) (result i64)
                (i64.or
                    (i64.shl (i64.const 1024) (i64.const 32))
                    (i64.const 38)))
        )
        "#
    }

    #[test]
    fn wasm_call_uses_the_bounded_abi() {
        let manifest = wat_manifest(echo_wat());
        let wasm = manifest.wasm.clone().unwrap();
        let plugin = WasmPlugin::from_bytes(manifest, wasm, echo_wat().as_bytes()).unwrap();
        let response = plugin.call(PluginRequest::new("describe")).unwrap();
        assert!(response.ok);
        assert_eq!(response.output["hello"], "world");
    }

    #[test]
    fn hash_mismatch_is_rejected() {
        let mut manifest = wat_manifest(echo_wat());
        manifest.wasm.as_mut().unwrap().sha256 = "0".repeat(64);
        let wasm = manifest.wasm.clone().unwrap();
        assert!(matches!(
            WasmPlugin::from_bytes(manifest, wasm, echo_wat().as_bytes()),
            Err(PluginError::HashMismatch)
        ));
    }

    #[test]
    fn registry_reads_manifests_without_loading_wasm() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("governance");
        std::fs::create_dir_all(&directory).unwrap();
        let wasm = echo_wat();
        let hash = hex::encode(Sha256::digest(wasm.as_bytes()));
        std::fs::write(
            directory.join("plugin.json"),
            format!(
                r#"{{"id":"governance","name":"Governance","version":"1.0.0","api":2,"wasm":{{"path":"plugin.wasm","sha256":"{hash}","exports":["dunlin_alloc","dunlin_free","dunlin_call"]}}}}"#
            ),
        )
        .unwrap();
        std::fs::write(directory.join("plugin.wasm"), wasm).unwrap();
        let registry = PluginRegistry::discover(root.path());
        assert_eq!(registry.plugins.len(), 1);
        assert!(registry.plugins[0].wasm_path.is_some());
        assert!(registry.plugins[0].load().is_ok());
    }

    #[test]
    fn plugin_response_errors_are_rejected() {
        let response = PluginResponse {
            ok: false,
            output: json!(null),
            warnings: Vec::new(),
            error: Some("bad object".into()),
        };
        assert!(response.into_output().is_err());
    }
}
