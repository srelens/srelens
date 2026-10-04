# Release-signing fixtures

Used by `apps/tui/tests/update_signature_tests.rs` to test how
`srelens-tui update` verifies a release signature (#448).

- `srelens-tui-0.15.0-SHA256SUMS.txt` and its `.asc` are copied byte for byte
  from the `srelens-v0.15.0` release. They were signed by the real release key
  in `KEYS`.
- Everything else was made by `make-fixtures.sh`, which uses gpg and a
  throwaway keyring in a container:

  ```sh
  docker run --rm -v "$PWD:/out" rust:1-bookworm sh /out/make-fixtures.sh
  ```

  Only public keys and signatures come out. No secret key is kept.

| File | Signed by | What it stands for |
| --- | --- | --- |
| `data.txt.untrusted.asc` | `untrusted-key.asc` | A good signature from a key that is not in `KEYS` |
| `data.txt.textmode.asc` | `untrusted-key.asc` | A text-mode signature rather than a binary one |
| `data.txt.sig-expired.asc` | `untrusted-key.asc` | Made in June 2020 with a one-day life |
| `data.txt.revoked.asc` | `revoked-key.asc` | Made before the key was revoked |
| `data.txt.expired.asc` | `expired-key.asc` | Made in June 2020 by a key that expired at the end of 2020 |
| `data.txt.extended.asc` | `extended-key.asc` | Made today by a key created in 2020 with a one-year life, then extended to never expire. The key is exported from a keyring that merged its old and new copies, so it carries both self-signatures. |

The directory is `-text` in `.gitattributes`. A signature covers exact bytes,
so a checkout that converted line endings would fail every check.
