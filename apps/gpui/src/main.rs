//! srelens on GPUI Kit — the resource list, as its own binary.
//!
//! This is the experiment `docs/design/gpui-kit-feasibility.md` asked for: one
//! screen, built against the existing Rust crates rather than through the
//! Tauri boundary, to find out what a migration would actually cost. It is
//! not wired into the shipping app and nothing in the shipping app depends on
//! it.
//!
//! Bootstrap follows the toolkit's own rules: `gpui_kit::init` before anything
//! else, one `Root` at the first level of the window, the application service
//! (`KubeBridge`) set as a global before the first view is created.

mod kube_bridge;
mod pods_table;
mod workspace;

use gpui_kit::component::Root;
use gpui_kit::*;

use kube_bridge::KubeBridge;
use workspace::Workspace;

fn main() {
    // Two rustls crypto providers end up in this process — kube-rs brings
    // `ring`, GPUI's HTTP client brings `aws-lc-rs` — and rustls will not
    // choose between them: it panics on the first TLS handshake, which was on
    // the kube thread, and took the pod list down without a word in the
    // window. `crates/kube` owns the choice and makes it before every client
    // it builds; this call just makes it before anything else in this binary
    // could touch TLS first.
    srelens_kube::connect::ensure_crypto_provider();

    let bridge = match KubeBridge::new() {
        Ok(bridge) => bridge,
        Err(error) => {
            eprintln!("srelens-gpui: could not start the kube runtime: {error}");
            std::process::exit(1);
        }
    };

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_global(bridge);

            let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("srelens".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(720.), px(480.))),
                ..Default::default()
            };

            cx.spawn(async move |cx| {
                cx.open_window(options, |window, cx| {
                    let workspace = cx.new(|cx| Workspace::new(window, cx));
                    cx.new(|cx| Root::new(workspace, window, cx))
                })
                .expect("failed to open the window");
            })
            .detach();
        });
}
