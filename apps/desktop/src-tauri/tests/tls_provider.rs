//! The desktop's HTTP clients build in a process that has done no TLS yet.
//!
//! reqwest is built with `rustls-no-provider`, so building a client before a
//! process default is installed panics ("No rustls crypto provider is
//! configured"). Inside a Tauri command that panic means the invoke never
//! resolves: the native agent's first turn hung until a cluster call had
//! installed one as a side effect. `main` installs it up front instead.
//!
//! Its own test binary, so nothing else in the process touched TLS first.

use srelens_llm::{HttpProvider, ProviderConfig, ProviderKind};

#[test]
fn http_clients_build_after_the_startup_install() {
    srelens_desktop_lib::install_tls_provider();

    // The native agent's client (`chat_send`, `llm_list_models`).
    HttpProvider::new(ProviderConfig {
        kind: ProviderKind::OpenAiCompatible,
        api_key: "test-key".into(),
        base_url: "http://127.0.0.1:1".into(),
        model: "fake-model".into(),
        max_tokens: 256,
    });
    // The Toolbox download client (`start_tool_install`).
    reqwest::blocking::Client::builder()
        .build()
        .expect("toolbox client builds");
}
