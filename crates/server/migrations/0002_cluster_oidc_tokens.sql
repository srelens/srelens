CREATE TABLE cluster_oidc_tokens (
    user_id        INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    oidc_key       TEXT NOT NULL,
    id_token_ct    BLOB NOT NULL,
    id_token_nonce BLOB NOT NULL,
    refresh_ct     BLOB,
    refresh_nonce  BLOB,
    expires_at     INTEGER NOT NULL,
    PRIMARY KEY (user_id, oidc_key)
);
