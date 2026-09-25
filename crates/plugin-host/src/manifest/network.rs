//! Brokered HTTP (#568): the scoped `network.http` permission, the hosts it
//! may name, and the allowlist the host resolves from them.
//!
//! An app never opens a connection. It declares `network.http` with the hosts
//! it may reach — names, subdomain wildcards, or `${settings.<id>}` for a `url`
//! setting a person fills in — and the host's broker makes each request,
//! holding the URL and every redirect to [`Manifest::network_allowlist`].
use super::{unique, Manifest};
use crate::{ValidationCode as Code, ValidationErrors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use srelens_capability::settings::{self, SettingType};
use std::net::{Ipv4Addr, Ipv6Addr};

/// The one capability granted with a scope.
pub const NETWORK_HTTP: &str = "network.http";
/// Most hosts one `network.http` permission may list.
pub const MAX_NETWORK_HOSTS: usize = 16;

/// One `permissions` entry: a host capability's id, or `network.http` with the
/// hosts it may reach.
///
/// A plain entry is stored as the string it was written as, so a manifest
/// signed before scoped permissions existed still round-trips to its bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Permission {
    /// A host capability the manifest binds, by id.
    Capability(String),
    /// A capability granted with a scope: today only `network.http`.
    Scoped(ScopedPermission),
}

/// `{"capability": "network.http", "hosts": [...]}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScopedPermission {
    pub capability: String,
    /// Where the capability may reach: `host`, `host:port`, `*.example.com`
    /// (one subdomain label), an IP literal (`[::1]` for IPv6), or
    /// `${settings.<id>}` naming a `url` setting, whose host and port are read
    /// from the saved value on every request.
    pub hosts: Vec<String>,
}

impl Permission {
    /// The capability this entry grants, which is what a grant names.
    pub fn capability(&self) -> &str {
        match self {
            Self::Capability(id) => id,
            Self::Scoped(scoped) => &scoped.capability,
        }
    }

    /// The hosts a scoped entry lists; empty for a plain one.
    pub fn hosts(&self) -> &[String] {
        match self {
            Self::Capability(_) => &[],
            Self::Scoped(scoped) => &scoped.hosts,
        }
    }
}

impl PartialEq<str> for Permission {
    fn eq(&self, other: &str) -> bool {
        self.capability() == other
    }
}

impl From<&str> for Permission {
    fn from(id: &str) -> Self {
        Self::Capability(id.to_owned())
    }
}

impl From<String> for Permission {
    fn from(id: String) -> Self {
        Self::Capability(id)
    }
}

/// Written by hand rather than as an untagged derive, so a scoped entry's own
/// mistakes are reported as themselves — an unknown field by name, at its
/// path — instead of "did not match any variant".
impl<'de> Deserialize<'de> for Permission {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entry;
        impl<'de> serde::de::Visitor<'de> for Entry {
            type Value = Permission;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a host capability id, or {\"capability\", \"hosts\"} for network.http")
            }
            fn visit_str<E: serde::de::Error>(self, id: &str) -> Result<Permission, E> {
                Ok(Permission::Capability(id.to_owned()))
            }
            fn visit_string<E: serde::de::Error>(self, id: String) -> Result<Permission, E> {
                Ok(Permission::Capability(id))
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                map: M,
            ) -> Result<Permission, M::Error> {
                ScopedPermission::deserialize(serde::de::value::MapAccessDeserializer::new(map))
                    .map(Permission::Scoped)
            }
        }
        deserializer.deserialize_any(Entry)
    }
}

/// One host an app may reach, as a literal `hosts` entry or a saved URL gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRule {
    host: Host,
    /// `None` is the default port of the request's scheme.
    port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Host {
    /// A DNS name, lowercase ASCII.
    Name(String),
    /// `*.` then a DNS name of two labels or more: exactly one label before it.
    Subdomain(String),
    V4(Ipv4Addr),
    V6(Ipv6Addr),
}

const HOST_RULE: &str = "Write a host as `name`, `name:port`, `*.example.com` (one subdomain label), an IP address (`[::1]` for IPv6), or `${settings.<id>}` for a url setting: lowercase ASCII, with no scheme, path or trailing dot";

/// A DNS name as the URL parser writes one: lowercase ASCII labels of letters,
/// digits and `-`, 1–63 characters each, not starting or ending with `-`, and a
/// last label that is not all digits (that is an IP address, or a typo of one).
fn dns_name(name: &str) -> bool {
    let labels: Vec<&str> = name.split('.').collect();
    name.len() <= 253
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
        && labels
            .last()
            .is_some_and(|last| !last.bytes().all(|b| b.is_ascii_digit()))
}

impl HostRule {
    /// A literal `hosts` entry, or why it is not one.
    pub fn parse(entry: &str) -> Result<Self, String> {
        let (host, port) = if let Some(rest) = entry.strip_prefix('[') {
            let (address, after) = rest.split_once(']').ok_or(HOST_RULE)?;
            let address: Ipv6Addr = address.parse().map_err(|_| HOST_RULE)?;
            let port = match after {
                "" => None,
                _ => Some(after.strip_prefix(':').ok_or(HOST_RULE)?),
            };
            (Host::V6(address), port)
        } else {
            let (name, port) = match entry.split_once(':') {
                Some((name, port)) => (name, Some(port)),
                None => (entry, None),
            };
            let host = if let Some(parent) = name.strip_prefix("*.") {
                if !dns_name(parent) || parent.split('.').count() < 2 {
                    return Err(HOST_RULE.into());
                }
                Host::Subdomain(parent.to_owned())
            } else if let Ok(address) = name.parse::<Ipv4Addr>() {
                Host::V4(address)
            } else if dns_name(name) {
                Host::Name(name.to_owned())
            } else {
                return Err(HOST_RULE.into());
            };
            (host, port)
        };
        // Digits only, with no sign or leading zero, so one port has one spelling.
        let port = port
            .map(|text| {
                text.parse::<u16>()
                    .ok()
                    .filter(|port| *port != 0 && port.to_string() == text)
                    .ok_or(HOST_RULE)
            })
            .transpose()?;
        Ok(Self { host, port })
    }

    /// The host and port of a saved URL: exactly that host, at its port.
    pub fn from_url(url: &url::Url) -> Option<Self> {
        let host = match url.host()? {
            url::Host::Domain(name) => Host::Name(name.to_owned()),
            url::Host::Ipv4(address) => Host::V4(address),
            url::Host::Ipv6(address) => Host::V6(address),
        };
        Some(Self {
            host,
            port: url.port(),
        })
    }

    /// Whether `url` is on this host, at this port.
    pub fn allows(&self, url: &url::Url) -> bool {
        let port = match self.port {
            None => url.port().is_none(),
            Some(port) => url.port_or_known_default() == Some(port),
        };
        port && match (&self.host, url.host()) {
            (Host::Name(name), Some(url::Host::Domain(host))) => host == name,
            (Host::Subdomain(parent), Some(url::Host::Domain(host))) => host
                .strip_suffix(parent.as_str())
                .and_then(|label| label.strip_suffix('.'))
                .is_some_and(|label| !label.is_empty() && !label.contains('.')),
            (Host::V4(address), Some(url::Host::Ipv4(host))) => *address == host,
            (Host::V6(address), Some(url::Host::Ipv6(host))) => *address == host,
            _ => false,
        }
    }
}

impl Manifest {
    /// The capability ids this manifest requests: what an install grants.
    pub fn permission_names(&self) -> Vec<String> {
        self.permissions
            .iter()
            .map(|permission| permission.capability().to_owned())
            .collect()
    }

    /// The hosts its `network.http` permission lists, as written.
    pub fn network_hosts(&self) -> &[String] {
        self.permissions
            .iter()
            .find(|permission| permission.capability() == NETWORK_HTTP)
            .map_or(&[], Permission::hosts)
    }

    /// Every host `network.http` may reach for this app with `settings` saved:
    /// each literal entry, and each `url` setting's saved value (else its
    /// default). A setting with neither adds nothing, and so does a value its
    /// declaration refuses — a hand-edited inventory gains nothing here.
    ///
    /// Resolved again for every request, so a changed setting moves the
    /// allowlist with it, and nothing it allowed before stays allowed.
    pub fn network_allowlist(&self, settings: &Map<String, Value>) -> Vec<HostRule> {
        self.network_hosts()
            .iter()
            .filter_map(
                |entry| match settings::reference(&Value::String(entry.clone())) {
                    None => HostRule::parse(entry).ok(),
                    Some(reference) => {
                        let setting = self.setting(reference.ok()?)?;
                        if setting.setting_type != SettingType::Url {
                            return None;
                        }
                        let value = setting.effective(settings.get(&setting.id))?;
                        setting.check_value(value).ok()?;
                        HostRule::from_url(&url::Url::parse(value.as_str()?).ok()?)
                    }
                },
            )
            .collect()
    }
}

/// The rules for `permissions` entries beyond their names: `network.http` is
/// scoped, nothing else is, and every host it lists is one [`HostRule`] reads
/// or a whole reference to a declared `url` setting.
pub(super) fn permission_problems(manifest: &Manifest, problems: &mut ValidationErrors) {
    for (index, permission) in manifest.permissions.iter().enumerate() {
        let at = format!("permissions[{index}]");
        let scoped = match permission {
            Permission::Capability(id) if id == NETWORK_HTTP => {
                problems.push(
                    Code::InvalidValue,
                    at,
                    "network.http lists the hosts it may reach: {\"capability\":\"network.http\",\"hosts\":[…]}",
                );
                continue;
            }
            Permission::Capability(_) => continue,
            Permission::Scoped(scoped) => scoped,
        };
        if scoped.capability != NETWORK_HTTP {
            problems.push(
                Code::InvalidField,
                format!("{at}.hosts"),
                "Only network.http is granted with hosts",
            );
            continue;
        }
        if scoped.hosts.is_empty() || scoped.hosts.len() > MAX_NETWORK_HOSTS {
            problems.push(
                Code::InvalidValue,
                format!("{at}.hosts"),
                format!("List 1–{MAX_NETWORK_HOSTS} hosts"),
            );
        }
        unique(
            problems,
            scoped
                .hosts
                .iter()
                .enumerate()
                .map(|(position, host)| (format!("{at}.hosts[{position}]"), host.as_str())),
        );
        for (position, host) in scoped.hosts.iter().enumerate() {
            let path = format!("{at}.hosts[{position}]");
            if let Err(why) = host_problem(manifest, host) {
                problems.push(Code::InvalidValue, path, why);
            }
        }
    }
}

/// Why `entry` cannot be a host of `manifest`'s `network.http` permission.
fn host_problem(manifest: &Manifest, entry: &str) -> Result<(), String> {
    let entry = Value::String(entry.to_owned());
    let id = match settings::reference(&entry) {
        None => return HostRule::parse(entry.as_str().unwrap_or_default()).map(|_| ()),
        Some(reference) => reference?,
    };
    match manifest.setting(id) {
        None => Err(format!("No setting \"{id}\" is declared")),
        Some(setting) if setting.setting_type != SettingType::Url => Err(format!(
            "A host is read from a url setting, and \"{id}\" is not one"
        )),
        Some(_) => Ok(()),
    }
}
