//! Fill new native hooks' placeholder bodies in the hand-written source.
pub fn run() -> Result<(), String> {
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let root = root
        .ancestors()
        .find(|path| path.join("native-hooks.json").is_file())
        .ok_or("run xtask from within the Horizon checkout")?;
    horizon_mod_api_gen::fill_hooks(root)
}

#[cfg(test)]
#[path = "mod_api_tests.rs"]
mod dispatch_tests;
