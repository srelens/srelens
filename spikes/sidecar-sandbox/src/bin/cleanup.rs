//! Remove what a spike run leaves behind: on Windows, the AppContainer profile the harness
//! registers (`DeleteAppContainerProfile`). Nothing to do elsewhere.

fn main() {
    match sidecar_sandbox_spike::cleanup() {
        Ok(()) => println!("cleaned up"),
        Err(e) => {
            eprintln!("cleanup failed: {e}");
            std::process::exit(1);
        }
    }
}
