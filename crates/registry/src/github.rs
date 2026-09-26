//! Why a rollout happened, from GitHub: the commits between two Argo syncs
//! that touched the app's path, and the pull requests they came from.
//!
//! Argo syncs to the head of a branch, so the synced SHA alone names only the
//! last commit of whatever merged since the previous sync — and in a repo
//! holding many apps, possibly a commit to another app's folder. The range
//! `previous..revision`, filtered by path, is the answer.
//!
//! github.com only. A token from `GITHUB_TOKEN` or `GH_TOKEN` is sent to
//! [`API_BASE`] and nowhere else: the host is fixed here, never taken from
//! the caller's `repoUrl`.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use srelens_capability::{Annotations, Capability, CapabilityError};

pub const API_BASE: &str = "https://api.github.com";

/// Commits whose pull requests are looked up, newest first. A range larger
/// than this is marked truncated rather than paged through.
const MAX_PR_LOOKUPS: usize = 20;
/// One page of path-filtered history; GitHub's maximum.
const PATH_PAGE: usize = 100;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(20);

/// A github.com repository, `owner/name`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GitHubRepo {
    pub owner: String,
    pub name: String,
}

impl GitHubRepo {
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// The github.com repository an Argo `repoURL` names, in any of the forms
/// Argo accepts: `https://github.com/o/r(.git)`, `git@github.com:o/r.git`,
/// `ssh://git@github.com/o/r.git`. `None` for every other host, so nothing
/// is ever requested from one.
pub fn parse_github_repo(url: &str) -> Option<GitHubRepo> {
    let url = url.trim();
    let path = if let Some(rest) = url.strip_prefix("git@github.com:") {
        rest
    } else {
        let no_scheme = ["https://", "http://", "ssh://", "git://"]
            .iter()
            .find_map(|s| url.strip_prefix(s))?;
        let (authority, path) = no_scheme.split_once('/')?;
        let host = authority.rsplit('@').next()?.split(':').next()?;
        if !host.eq_ignore_ascii_case("github.com") && !host.eq_ignore_ascii_case("www.github.com")
        {
            return None;
        }
        path
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    let valid = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (parts.next().is_none() && valid(owner) && valid(name)).then(|| GitHubRepo {
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

fn is_commit_sha(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit())
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CauseCommit {
    pub sha: String,
    /// The first line of the commit message.
    pub subject: String,
    /// GitHub login when the commit is linked to an account, else the git
    /// author name.
    pub author: String,
    #[serde(rename = "isBot")]
    pub is_bot: bool,
    #[serde(rename = "htmlUrl")]
    pub html_url: String,
    #[serde(rename = "committedAt")]
    pub committed_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CausePull {
    pub number: u64,
    pub title: String,
    #[serde(rename = "htmlUrl")]
    pub html_url: String,
    pub user: String,
    #[serde(rename = "isBot")]
    pub is_bot: bool,
    #[serde(rename = "mergedAt")]
    pub merged_at: Option<String>,
    /// The commits in [`RolloutCause::commits`] this pull request brought.
    pub commits: Vec<String>,
}

/// What GitHub says caused a rollout.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RolloutCause {
    /// `owner/name`.
    pub repo: String,
    pub revision: String,
    #[serde(rename = "previousRevision")]
    pub previous_revision: Option<String>,
    pub path: String,
    /// The sync went back to an older revision; `commits` are the ones it
    /// undid.
    #[serde(rename = "rolledBack")]
    pub rolled_back: bool,
    /// Commits in the range that touched `path` (all of them when `path` is
    /// empty), newest first. With no previous revision, the synced commit.
    pub commits: Vec<CauseCommit>,
    pub pulls: Vec<CausePull>,
    /// Commits in `commits` that no merged pull request brought.
    #[serde(rename = "directCommits")]
    pub direct_commits: Vec<String>,
    /// Commits in the range that did not touch `path`: other apps' changes.
    #[serde(rename = "otherCommits")]
    pub other_commits: usize,
    /// GitHub returned less than the whole range, or more commits matched
    /// than were looked up; what is listed is true but not complete.
    pub truncated: bool,
    #[serde(rename = "compareUrl")]
    pub compare_url: Option<String>,
}

/// Why GitHub could not answer. Each says what failed, never that there was
/// nothing to find.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubError {
    NotGitHub(String),
    NotACommit(String),
    AuthRejected,
    NotFoundOrNoAccess { has_token: bool },
    RateLimited { has_token: bool },
    Forbidden(String),
    Network(String),
    Unexpected { status: u16, message: String },
}

impl std::fmt::Display for GitHubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotGitHub(url) => write!(f, "not a github.com repository: {url}"),
            Self::NotACommit(rev) => write!(f, "{rev} is not a git commit SHA"),
            Self::AuthRejected => write!(f, "GitHub rejected the token in GITHUB_TOKEN/GH_TOKEN"),
            Self::NotFoundOrNoAccess { has_token: false } => write!(
                f,
                "not found on GitHub, or the repository is private and no GITHUB_TOKEN is set"
            ),
            Self::NotFoundOrNoAccess { has_token: true } => write!(
                f,
                "not found on GitHub, or the token cannot read this repository"
            ),
            Self::RateLimited { has_token: false } => write!(
                f,
                "GitHub rate limit reached; set GITHUB_TOKEN for a higher limit"
            ),
            Self::RateLimited { has_token: true } => {
                write!(f, "GitHub rate limit reached for this token")
            }
            Self::Forbidden(msg) => write!(f, "GitHub refused: {msg}"),
            Self::Network(e) => write!(f, "could not reach GitHub: {e}"),
            Self::Unexpected { status, message } => {
                write!(f, "GitHub answered {status}: {message}")
            }
        }
    }
}

impl std::error::Error for GitHubError {}

fn token_from_env() -> Option<String> {
    ["GITHUB_TOKEN", "GH_TOKEN"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .map(|t| t.trim().to_string())
        .find(|t| !t.is_empty())
}

struct GitHub {
    base: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl GitHub {
    fn new(base: &str, token: Option<String>) -> Result<Self, GitHubError> {
        // Rustls needs one provider; the first GitHub call may come before
        // any kube client installed it.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut builder = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(TOTAL_TIMEOUT)
            .user_agent("srelens");
        if base.starts_with("http://127.0.0.1") {
            builder = builder.no_proxy();
        }
        let http = builder
            .build()
            .map_err(|e| GitHubError::Network(e.to_string()))?;
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            token,
            http,
        })
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, GitHubError> {
        let mut url = url::Url::parse(&format!("{}{path}", self.base))
            .map_err(|e| GitHubError::Network(e.to_string()))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let mut req = self
            .http
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| GitHubError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let exhausted = resp
            .headers()
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            == Some("0");
        let body = resp
            .bytes()
            .await
            .map_err(|e| GitHubError::Network(e.to_string()))?;
        let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let message = || json["message"].as_str().unwrap_or("no message").to_string();
        let has_token = self.token.is_some();
        match status {
            200..=299 => Ok(json),
            401 => Err(GitHubError::AuthRejected),
            404 => Err(GitHubError::NotFoundOrNoAccess { has_token }),
            429 => Err(GitHubError::RateLimited { has_token }),
            403 if exhausted => Err(GitHubError::RateLimited { has_token }),
            403 => Err(GitHubError::Forbidden(message())),
            _ => Err(GitHubError::Unexpected {
                status,
                message: message(),
            }),
        }
    }

    async fn rollout_cause(
        &self,
        repo: &GitHubRepo,
        revision: &str,
        previous: Option<&str>,
        path: &str,
    ) -> Result<RolloutCause, GitHubError> {
        for rev in std::iter::once(revision).chain(previous) {
            if !is_commit_sha(rev) {
                return Err(GitHubError::NotACommit(rev.to_string()));
            }
        }
        let slug = repo.slug();
        let mut cause = RolloutCause {
            repo: slug.clone(),
            revision: revision.to_string(),
            previous_revision: previous.map(str::to_string),
            path: path.to_string(),
            ..Default::default()
        };

        let kept: Vec<Value> = match previous {
            None => vec![
                self.get(&format!("/repos/{slug}/commits/{revision}"), &[])
                    .await?,
            ],
            Some(prev) => {
                let mut cmp = self
                    .get(&format!("/repos/{slug}/compare/{prev}...{revision}"), &[])
                    .await?;
                let mut head = revision;
                if cmp["status"] == "behind" {
                    cause.rolled_back = true;
                    head = prev;
                    cmp = self
                        .get(&format!("/repos/{slug}/compare/{revision}...{prev}"), &[])
                        .await?;
                }
                cause.compare_url = cmp["html_url"].as_str().map(str::to_string);
                let mut in_range: Vec<Value> =
                    cmp["commits"].as_array().cloned().unwrap_or_default();
                let total = cmp["total_commits"]
                    .as_u64()
                    .unwrap_or(in_range.len() as u64);
                cause.truncated |= total > in_range.len() as u64;
                in_range.reverse();

                if path.is_empty() {
                    in_range
                } else {
                    let per_page = PATH_PAGE.to_string();
                    let touched = self
                        .get(
                            &format!("/repos/{slug}/commits"),
                            &[("sha", head), ("path", path), ("per_page", &per_page)],
                        )
                        .await?;
                    let touched = touched.as_array().cloned().unwrap_or_default();
                    let shas: HashSet<&str> =
                        touched.iter().filter_map(|c| c["sha"].as_str()).collect();
                    // A full page whose oldest entry is still in the range
                    // may have more of the range behind it.
                    let range_shas: HashSet<&str> =
                        in_range.iter().filter_map(|c| c["sha"].as_str()).collect();
                    if touched.len() == PATH_PAGE
                        && touched
                            .last()
                            .and_then(|c| c["sha"].as_str())
                            .is_some_and(|s| range_shas.contains(s))
                    {
                        cause.truncated = true;
                    }
                    let (kept, other): (Vec<Value>, Vec<Value>) = in_range
                        .into_iter()
                        .partition(|c| c["sha"].as_str().is_some_and(|s| shas.contains(s)));
                    cause.other_commits = other.len();
                    kept
                }
            }
        };

        cause.commits = kept.iter().map(commit_of).collect();
        if cause.commits.len() > MAX_PR_LOOKUPS {
            cause.truncated = true;
        }
        let mut by_number: HashMap<u64, usize> = HashMap::new();
        for commit in cause.commits.iter().take(MAX_PR_LOOKUPS) {
            let prs = self
                .get(&format!("/repos/{slug}/commits/{}/pulls", commit.sha), &[])
                .await?;
            let merged: Vec<&Value> = prs
                .as_array()
                .into_iter()
                .flatten()
                .filter(|p| !p["merged_at"].is_null())
                .collect();
            if merged.is_empty() {
                cause.direct_commits.push(commit.sha.clone());
            }
            for p in merged {
                let Some(number) = p["number"].as_u64() else {
                    continue;
                };
                let idx = *by_number.entry(number).or_insert_with(|| {
                    cause.pulls.push(pull_of(p));
                    cause.pulls.len() - 1
                });
                cause.pulls[idx].commits.push(commit.sha.clone());
            }
        }
        Ok(cause)
    }
}

fn is_bot_account(account: &Value) -> bool {
    account["type"] == "Bot"
        || account["login"]
            .as_str()
            .is_some_and(|l| l.ends_with("[bot]"))
}

fn commit_of(c: &Value) -> CauseCommit {
    let git = &c["commit"];
    let login = c["author"]["login"].as_str();
    let name = git["author"]["name"].as_str().unwrap_or_default();
    CauseCommit {
        sha: c["sha"].as_str().unwrap_or_default().to_string(),
        subject: git["message"]
            .as_str()
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or_default()
            .to_string(),
        author: login.unwrap_or(name).to_string(),
        is_bot: is_bot_account(&c["author"]) || name.ends_with("[bot]"),
        html_url: c["html_url"].as_str().unwrap_or_default().to_string(),
        committed_at: git["committer"]["date"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    }
}

fn pull_of(p: &Value) -> CausePull {
    CausePull {
        number: p["number"].as_u64().unwrap_or_default(),
        title: p["title"].as_str().unwrap_or_default().to_string(),
        html_url: p["html_url"].as_str().unwrap_or_default().to_string(),
        user: p["user"]["login"].as_str().unwrap_or_default().to_string(),
        is_bot: is_bot_account(&p["user"]),
        merged_at: p["merged_at"].as_str().map(str::to_string),
        commits: Vec::new(),
    }
}

type CacheKey = (String, String, Option<String>, String);

/// Answers already fetched. Both revisions are fixed commits, so an answer
/// never goes stale.
fn cache() -> &'static Mutex<HashMap<CacheKey, RolloutCause>> {
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, RolloutCause>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Why the sync from `previous` to `revision` of `repo_url` rolled out:
/// the commits in between that touched `path`, and their pull requests.
pub async fn rollout_cause(
    repo_url: &str,
    revision: &str,
    previous: Option<&str>,
    path: &str,
) -> Result<RolloutCause, GitHubError> {
    let repo =
        parse_github_repo(repo_url).ok_or_else(|| GitHubError::NotGitHub(repo_url.to_string()))?;
    let key = (
        repo.slug(),
        revision.to_string(),
        previous.map(str::to_string),
        path.to_string(),
    );
    if let Some(hit) = cache().lock().ok().and_then(|c| c.get(&key).cloned()) {
        return Ok(hit);
    }
    let cause = GitHub::new(API_BASE, token_from_env())?
        .rollout_cause(&repo, revision, previous, path)
        .await?;
    if let Ok(mut c) = cache().lock() {
        c.insert(key, cause.clone());
    }
    Ok(cause)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RolloutCauseIn {
    /// The Argo source `repoURL` (github.com only).
    #[serde(rename = "repoUrl")]
    pub repo_url: String,
    /// The synced commit SHA.
    pub revision: String,
    /// The commit SHA of the sync before it; omit for an app's first sync.
    #[serde(default, rename = "previousRevision")]
    pub previous_revision: Option<String>,
    /// The app's source path; commits outside it are counted, not listed.
    #[serde(default)]
    pub path: String,
}

/// `github.rolloutCause` — the pull requests behind an Argo sync.
pub fn rollout_cause_capability() -> Capability {
    Capability::typed::<RolloutCauseIn, RolloutCause, _, _>(
        "github.rolloutCause",
        "why an Argo sync rolled out: the github.com commits between the previous and the synced revision that touched the app's path, and the pull requests they came from",
        Annotations::READ_ONLY,
        |input: RolloutCauseIn| async move {
            rollout_cause(
                &input.repo_url,
                &input.revision,
                input.previous_revision.as_deref(),
                &input.path,
            )
            .await
            .map_err(|e| CapabilityError::Handler(e.to_string()))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    const PREV: &str = "1111111111111111111111111111111111111111";
    const HEAD: &str = "3333333333333333333333333333333333333333";
    const MID: &str = "2222222222222222222222222222222222222222";

    #[test]
    fn github_repo_urls_parse_in_every_form_argo_accepts() {
        let acme = Some(GitHubRepo {
            owner: "acme".into(),
            name: "deploy".into(),
        });
        for url in [
            "https://github.com/acme/deploy",
            "https://github.com/acme/deploy.git",
            "https://github.com/acme/deploy/",
            "git@github.com:acme/deploy.git",
            "ssh://git@github.com/acme/deploy.git",
            "https://user@github.com/acme/deploy.git",
        ] {
            assert_eq!(parse_github_repo(url), acme, "{url}");
        }
        for url in [
            "https://gitlab.com/acme/deploy.git",
            "https://github.acme.corp/acme/deploy.git",
            "git@gitlab.com:acme/deploy.git",
            "https://github.com.evil.io/acme/deploy",
            "https://charts.acme.io",
            "https://github.com/acme",
            "https://github.com/acme/deploy/tree/main",
            "",
        ] {
            assert_eq!(parse_github_repo(url), None, "{url}");
        }
    }

    #[test]
    fn the_input_reads_the_callers_camel_case_and_rejects_snake_case() {
        let ok: RolloutCauseIn = serde_json::from_value(serde_json::json!({
            "repoUrl": "https://github.com/acme/deploy",
            "revision": HEAD,
            "previousRevision": PREV,
            "path": "apps/shop"
        }))
        .unwrap();
        assert_eq!(ok.previous_revision.as_deref(), Some(PREV));
        let wrong = serde_json::from_value::<RolloutCauseIn>(serde_json::json!({
            "repo_url": "https://github.com/acme/deploy",
            "revision": HEAD
        }));
        assert!(wrong.is_err(), "repo_url is not the wire name");
    }

    #[derive(Clone)]
    struct Reply {
        status: u16,
        body: Value,
        headers: Vec<(&'static str, &'static str)>,
    }

    fn ok(body: Value) -> Reply {
        Reply {
            status: 200,
            body,
            headers: vec![],
        }
    }

    struct Server {
        base: String,
        seen: Arc<Mutex<Vec<(String, Option<String>)>>>,
    }

    impl Server {
        fn targets(&self) -> Vec<String> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .map(|s| s.0.clone())
                .collect()
        }
    }

    /// HTTP/1.1 on 127.0.0.1, answering `answer(request target)`, one
    /// request per connection. Records each target and Authorization.
    async fn server(answer: impl Fn(&str) -> Reply + Send + Sync + 'static) -> Server {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let answer = Arc::new(answer);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (log, answer) = (log.clone(), answer.clone());
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    if reader.read_line(&mut line).await.is_err() {
                        return;
                    }
                    let target = line.split(' ').nth(1).unwrap_or_default().to_owned();
                    let mut auth = None;
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).await.unwrap_or(0) == 0 {
                            break;
                        }
                        let header = header.trim_end();
                        if header.is_empty() {
                            break;
                        }
                        if let Some((k, v)) = header.split_once(':') {
                            if k.eq_ignore_ascii_case("authorization") {
                                auth = Some(v.trim().to_owned());
                            }
                        }
                    }
                    log.lock().unwrap().push((target.clone(), auth));
                    let reply = answer(&target);
                    let body = reply.body.to_string();
                    let mut head = format!(
                        "HTTP/1.1 {} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n",
                        reply.status,
                        body.len()
                    );
                    for (k, v) in reply.headers {
                        head.push_str(&format!("{k}: {v}\r\n"));
                    }
                    head.push_str("\r\n");
                    let mut stream = reader.into_inner();
                    let _ = stream.write_all(head.as_bytes()).await;
                    let _ = stream.write_all(body.as_bytes()).await;
                });
            }
        });
        Server { base, seen }
    }

    fn commit(sha: &str, subject: &str, login: &str, kind: &str) -> Value {
        serde_json::json!({
            "sha": sha,
            "html_url": format!("https://github.com/acme/deploy/commit/{sha}"),
            "author": { "login": login, "type": kind },
            "commit": {
                "message": format!("{subject}\n\nbody text"),
                "author": { "name": login },
                "committer": { "date": "2026-09-26T10:00:00Z" }
            }
        })
    }

    fn pull(number: u64, title: &str, login: &str) -> Value {
        serde_json::json!({
            "number": number,
            "title": title,
            "html_url": format!("https://github.com/acme/deploy/pull/{number}"),
            "user": { "login": login, "type": if login.ends_with("[bot]") { "Bot" } else { "User" } },
            "merged_at": "2026-09-26T10:00:00Z"
        })
    }

    fn repo() -> GitHubRepo {
        parse_github_repo("https://github.com/acme/deploy").unwrap()
    }

    /// The range PREV..HEAD holds MID (another app's folder, by bob) and
    /// HEAD (this app, a human PR) plus a bot bump of this app.
    fn range_server() -> impl Fn(&str) -> Reply {
        |target: &str| {
            let bump = "4444444444444444444444444444444444444444";
            if target.starts_with(&format!("/repos/acme/deploy/compare/{PREV}...{HEAD}")) {
                ok(serde_json::json!({
                    "status": "ahead",
                    "html_url": "https://github.com/acme/deploy/compare/x...y",
                    "total_commits": 3,
                    "commits": [
                        commit(bump, "chore(deps): bump shop to 1.4.3", "renovate[bot]", "Bot"),
                        commit(MID, "feat(billing): new plan", "bob", "User"),
                        commit(HEAD, "fix(shop): bump checkout timeout", "alice", "User"),
                    ]
                }))
            } else if target.starts_with("/repos/acme/deploy/commits?") {
                assert!(target.contains(&format!("sha={HEAD}")), "{target}");
                assert!(target.contains("path=apps%2Fshop"), "{target}");
                ok(serde_json::json!([
                    commit(HEAD, "fix(shop): bump checkout timeout", "alice", "User"),
                    commit(
                        bump,
                        "chore(deps): bump shop to 1.4.3",
                        "renovate[bot]",
                        "Bot"
                    ),
                    commit(
                        "9999999999999999999999999999999999999999",
                        "older",
                        "carol",
                        "User"
                    ),
                ]))
            } else if target == format!("/repos/acme/deploy/commits/{HEAD}/pulls") {
                ok(serde_json::json!([pull(
                    1842,
                    "fix: bump checkout timeout",
                    "alice"
                )]))
            } else if target.starts_with("/repos/acme/deploy/commits/4444") {
                ok(serde_json::json!([pull(
                    1843,
                    "chore(deps): bump shop",
                    "renovate[bot]"
                )]))
            } else {
                Reply {
                    status: 500,
                    body: serde_json::json!({ "message": format!("unexpected {target}") }),
                    headers: vec![],
                }
            }
        }
    }

    #[tokio::test]
    async fn the_cause_is_the_prs_in_range_that_touched_the_app_path() {
        let srv = server(range_server()).await;
        let gh = GitHub::new(&srv.base, None).unwrap();
        let cause = gh
            .rollout_cause(&repo(), HEAD, Some(PREV), "apps/shop")
            .await
            .unwrap();

        let subjects: Vec<&str> = cause.commits.iter().map(|c| c.subject.as_str()).collect();
        assert_eq!(
            subjects,
            [
                "fix(shop): bump checkout timeout",
                "chore(deps): bump shop to 1.4.3"
            ],
            "newest first; bob's billing commit is another app's"
        );
        assert_eq!(cause.other_commits, 1);
        assert!(!cause.commits[0].is_bot);
        assert!(cause.commits[1].is_bot);
        let prs: Vec<(u64, bool)> = cause.pulls.iter().map(|p| (p.number, p.is_bot)).collect();
        assert_eq!(prs, [(1842, false), (1843, true)]);
        assert_eq!(cause.pulls[0].commits, [HEAD]);
        assert!(cause.direct_commits.is_empty());
        assert!(!cause.truncated && !cause.rolled_back);
        assert!(!srv
            .targets()
            .iter()
            .any(|t| t.contains(MID) && t.ends_with("/pulls")));
    }

    #[tokio::test]
    async fn in_a_repo_of_many_apps_an_unrelated_head_commit_is_not_the_answer() {
        let srv = server(|target: &str| {
            if target.contains("/compare/") {
                ok(serde_json::json!({
                    "status": "ahead", "total_commits": 2,
                    "commits": [
                        commit(MID, "fix(shop): the real change", "alice", "User"),
                        commit(HEAD, "docs(billing): typo", "bob", "User"),
                    ]
                }))
            } else if target.starts_with("/repos/acme/deploy/commits?") {
                ok(serde_json::json!([commit(
                    MID,
                    "fix(shop): the real change",
                    "alice",
                    "User"
                )]))
            } else if target.ends_with("/pulls") {
                ok(serde_json::json!([pull(7, "the real change", "alice")]))
            } else {
                Reply {
                    status: 500,
                    body: Value::Null,
                    headers: vec![],
                }
            }
        })
        .await;
        let gh = GitHub::new(&srv.base, None).unwrap();
        let cause = gh
            .rollout_cause(&repo(), HEAD, Some(PREV), "apps/shop")
            .await
            .unwrap();
        assert_eq!(cause.commits.len(), 1);
        assert_eq!(cause.commits[0].sha, MID);
        assert_eq!(cause.pulls[0].number, 7);
    }

    #[tokio::test]
    async fn a_commit_with_no_merged_pr_is_a_direct_commit() {
        let srv = server(|target: &str| {
            if target.ends_with("/pulls") {
                // Only an open PR carries it: not how it reached the branch.
                ok(serde_json::json!([{ "number": 9, "title": "wip", "merged_at": null, "user": {} }]))
            } else {
                ok(commit(HEAD, "hotfix", "carol", "User"))
            }
        })
        .await;
        let gh = GitHub::new(&srv.base, None).unwrap();
        let cause = gh
            .rollout_cause(&repo(), HEAD, None, "apps/shop")
            .await
            .unwrap();
        assert_eq!(cause.commits[0].subject, "hotfix");
        assert!(cause.pulls.is_empty());
        assert_eq!(cause.direct_commits, [HEAD]);
    }

    #[tokio::test]
    async fn a_sync_back_to_an_older_revision_lists_the_commits_it_undid() {
        let srv = server(|target: &str| {
            if target.contains(&format!("/compare/{HEAD}...{PREV}")) {
                ok(serde_json::json!({ "status": "behind", "total_commits": 0, "commits": [] }))
            } else if target.contains(&format!("/compare/{PREV}...{HEAD}")) {
                ok(serde_json::json!({
                    "status": "ahead", "total_commits": 1,
                    "commits": [commit(HEAD, "feat: the change undone", "alice", "User")]
                }))
            } else if target.ends_with("/pulls") {
                ok(serde_json::json!([pull(
                    5,
                    "feat: the change undone",
                    "alice"
                )]))
            } else {
                Reply {
                    status: 500,
                    body: Value::Null,
                    headers: vec![],
                }
            }
        })
        .await;
        let gh = GitHub::new(&srv.base, None).unwrap();
        // Synced PREV after HEAD: a rollback.
        let cause = gh
            .rollout_cause(&repo(), PREV, Some(HEAD), "")
            .await
            .unwrap();
        assert!(cause.rolled_back);
        assert_eq!(cause.commits[0].sha, HEAD);
        assert_eq!(cause.pulls[0].number, 5);
    }

    #[tokio::test]
    async fn a_range_larger_than_github_returns_is_marked_truncated() {
        let srv = server(|target: &str| {
            if target.contains("/compare/") {
                ok(serde_json::json!({
                    "status": "ahead", "total_commits": 400,
                    "commits": [commit(HEAD, "last", "alice", "User")]
                }))
            } else {
                ok(serde_json::json!([]))
            }
        })
        .await;
        let gh = GitHub::new(&srv.base, None).unwrap();
        let cause = gh
            .rollout_cause(&repo(), HEAD, Some(PREV), "")
            .await
            .unwrap();
        assert!(cause.truncated);
    }

    #[tokio::test]
    async fn failures_say_what_failed_and_never_look_like_no_changes() {
        let saml = "Resource protected by organization SAML enforcement";
        let reply = |status, headers| Reply {
            status,
            body: serde_json::json!({ "message": saml }),
            headers,
        };
        let cases: Vec<(Reply, Option<&str>, GitHubError)> = vec![
            (
                reply(404, vec![]),
                None,
                GitHubError::NotFoundOrNoAccess { has_token: false },
            ),
            (
                reply(404, vec![]),
                Some("t"),
                GitHubError::NotFoundOrNoAccess { has_token: true },
            ),
            (reply(401, vec![]), Some("t"), GitHubError::AuthRejected),
            (
                reply(403, vec![("x-ratelimit-remaining", "0")]),
                None,
                GitHubError::RateLimited { has_token: false },
            ),
            (
                reply(429, vec![]),
                Some("t"),
                GitHubError::RateLimited { has_token: true },
            ),
            (
                reply(403, vec![]),
                Some("t"),
                GitHubError::Forbidden(saml.into()),
            ),
        ];
        for (r, token, want) in cases {
            let srv = server(move |_| r.clone()).await;
            let gh = GitHub::new(&srv.base, token.map(str::to_string)).unwrap();
            let err = gh
                .rollout_cause(&repo(), HEAD, Some(PREV), "")
                .await
                .unwrap_err();
            assert_eq!(err, want);
        }
        assert!(GitHubError::NotFoundOrNoAccess { has_token: false }
            .to_string()
            .contains("private and no GITHUB_TOKEN"));
    }

    #[tokio::test]
    async fn the_token_goes_as_a_bearer_and_revisions_must_be_shas() {
        let srv = server(|_| ok(commit(HEAD, "x", "a", "User"))).await;
        let gh = GitHub::new(&srv.base, Some("s3cret".into())).unwrap();
        gh.rollout_cause(&repo(), HEAD, None, "").await.unwrap();
        let seen = srv.seen.lock().unwrap().clone();
        assert!(seen
            .iter()
            .all(|(_, auth)| auth.as_deref() == Some("Bearer s3cret")));

        let err = gh
            .rollout_cause(&repo(), "1.4.3", None, "")
            .await
            .unwrap_err();
        assert_eq!(err, GitHubError::NotACommit("1.4.3".into()));
        let err = gh
            .rollout_cause(&repo(), HEAD, Some("../../x"), "")
            .await
            .unwrap_err();
        assert_eq!(err, GitHubError::NotACommit("../../x".into()));
    }

    #[tokio::test]
    async fn a_non_github_repo_is_refused_before_any_request() {
        let err = rollout_cause("https://gitlab.com/acme/deploy.git", HEAD, None, "")
            .await
            .unwrap_err();
        assert_eq!(
            err,
            GitHubError::NotGitHub("https://gitlab.com/acme/deploy.git".into())
        );
    }
}
