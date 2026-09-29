//! Platform-agnostic simulation bridge (MuJoCo / ManiSkill / …).

pub mod assets;
pub mod backend;
pub mod protocol;

pub use assets::{
    default_catalog_path, list_builtins, list_builtins_status, load_catalog, resolve_model_spec,
    AssetCatalog, ResolvedAsset,
};
pub use backend::{
    default_worker_script, ensure_script_exists, workspace_python_root, BridgeConfig,
    BridgedSimBackend,
};
pub use protocol::{ClientMsg, ServerMsg, PROTOCOL_VERSION};
