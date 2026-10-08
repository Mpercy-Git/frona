//! Outbound guard for Web Push.
//!
//! A push subscription's endpoint is a URL the *user* chose, and the server
//! POSTs to it. Left unchecked that is a server-side request forgery primitive:
//! a subscription pointing at `https://169.254.169.254/...` or a service on the
//! internal network makes the server probe it, and the push service's reply is
//! echoed back in the delivery report. HTTPS alone excludes none of that.
//!
//! Two layers, because neither is enough alone:
//! - [`validate_endpoint`] rejects endpoints that are wrong on their face (bad
//!   scheme, credentials in the URL, `localhost`, an IP literal that is not
//!   public). It runs when a subscription is stored and again before each send.
//! - [`PublicOnlyResolver`] is the connection-time check. Every address a
//!   hostname resolves to is filtered, so a name that points at, or is later
//!   rebound to, a private address never gets a connection.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// True for an address a public push service could live at. Loopback, private,
/// link-local (including the cloud metadata address), CGNAT, multicast,
/// documentation, benchmarking and otherwise reserved ranges are not.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        // 100.64.0.0/10, carrier-grade NAT
        || (a == 100 && (64..=127).contains(&b))
        // 192.0.0.0/24, IETF protocol assignments
        || (a == 192 && b == 0 && c == 0)
        // 198.18.0.0/15, benchmarking
        || (a == 198 && (b == 18 || b == 19))
        // 240.0.0.0/4, reserved
        || a >= 240)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    // An IPv4 address carried inside IPv6 is judged as that IPv4 address.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let seg = ip.segments();
    // 64:ff9b::/96 (NAT64) and 2002::/16 (6to4) embed an IPv4 address.
    if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return is_public_v4(Ipv4Addr::new(
            (seg[6] >> 8) as u8,
            seg[6] as u8,
            (seg[7] >> 8) as u8,
            seg[7] as u8,
        ));
    }
    if seg[0] == 0x2002 {
        return is_public_v4(Ipv4Addr::new(
            (seg[1] >> 8) as u8,
            seg[1] as u8,
            (seg[2] >> 8) as u8,
            seg[2] as u8,
        ));
    }
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        // fc00::/7, unique local
        || (seg[0] & 0xfe00) == 0xfc00
        // fe80::/10, link-local; fec0::/10, deprecated site-local
        || (seg[0] & 0xffc0) == 0xfe80
        || (seg[0] & 0xffc0) == 0xfec0
        // 2001:db8::/32, documentation
        || (seg[0] == 0x2001 && seg[1] == 0x0db8))
}

/// Static checks on a push endpoint. Returns the parsed URL.
///
/// DNS is deliberately not consulted here: a name can be rebound between this
/// check and the connection, so the answer would only be advisory. The
/// connection-time check is [`PublicOnlyResolver`].
pub fn validate_endpoint(endpoint: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(endpoint).map_err(|_| "Invalid endpoint URL".to_string())?;
    if url.scheme() != "https" {
        return Err("Push endpoint must use HTTPS".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Push endpoint must not contain credentials".into());
    }
    match url.host() {
        None => return Err("Push endpoint has no host".into()),
        Some(url::Host::Ipv4(ip)) if !is_public_v4(ip) => {
            return Err("Push endpoint must not point at a private address".into());
        }
        Some(url::Host::Ipv6(ip)) if !is_public_v6(ip) => {
            return Err("Push endpoint must not point at a private address".into());
        }
        Some(url::Host::Domain(name)) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            let local = name == "localhost"
                || name.ends_with(".localhost")
                || name.ends_with(".local")
                || name.ends_with(".internal")
                || name.ends_with(".lan")
                || !name.contains('.');
            if local {
                return Err("Push endpoint must not point at a local host".into());
            }
        }
        Some(_) => {}
    }
    Ok(url)
}

/// DNS resolver for the push client that only ever yields public addresses.
/// A name with no public address fails to resolve, so no socket is opened.
///
/// reqwest does not consult the resolver for an IP literal; those are covered
/// by [`validate_endpoint`].
#[derive(Debug, Clone, Copy, Default)]
pub struct PublicOnlyResolver;

impl Resolve for PublicOnlyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<_> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|addr| is_public_ip(addr.ip()))
                .collect();
            if addrs.is_empty() {
                return Err(format!("{host} does not resolve to a public address").into());
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn private_and_reserved_addresses_are_not_public() {
        for s in [
            "127.0.0.1",
            "10.0.0.5",
            "172.16.0.1",
            "172.31.255.254",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "240.0.0.1",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd12:3456::1",
            "ff02::1",
            "2001:db8::1",
            // IPv4 hidden in IPv6
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "64:ff9b::7f00:1",
            "2002:7f00:1::",
        ] {
            assert!(!is_public_ip(ip(s)), "{s} must not be public");
        }
    }

    #[test]
    fn ordinary_addresses_are_public() {
        for s in [
            "8.8.8.8",
            "142.250.80.46",
            "172.15.0.1",
            "172.32.0.1",
            "100.63.0.1",
            "100.128.0.1",
            "2607:f8b0:4005::200e",
            "::ffff:8.8.8.8",
        ] {
            assert!(is_public_ip(ip(s)), "{s} must be public");
        }
    }

    #[test]
    fn real_push_service_endpoints_pass() {
        for endpoint in [
            "https://fcm.googleapis.com/fcm/send/abc",
            "https://updates.push.services.mozilla.com/wpush/v2/xyz",
            "https://web.push.apple.com/QGx9",
            "https://wns2-par02p.notify.windows.com/w/?token=abc",
            "https://ntfy.example.org:8443/up?x=1",
        ] {
            assert!(validate_endpoint(endpoint).is_ok(), "{endpoint}");
        }
    }

    #[test]
    fn endpoints_that_are_wrong_on_their_face_are_refused() {
        for endpoint in [
            "http://fcm.googleapis.com/fcm/send/abc",
            "https://user:pass@fcm.googleapis.com/x",
            "https://localhost/x",
            "https://app.localhost/x",
            "https://printer.local/x",
            "https://db.internal/x",
            "https://intranet/x",
            "https://127.0.0.1/x",
            "https://169.254.169.254/latest/meta-data",
            "https://10.0.0.1:8080/x",
            "https://[::1]/x",
            "https://[::ffff:10.0.0.1]/x",
            "ftp://fcm.googleapis.com/x",
            "not a url",
            "https:///nohost",
        ] {
            assert!(validate_endpoint(endpoint).is_err(), "{endpoint}");
        }
    }

    #[tokio::test]
    async fn the_resolver_refuses_a_name_that_only_has_private_addresses() {
        let resolver = PublicOnlyResolver;
        let name: Name = "localhost".parse().unwrap();
        assert!(resolver.resolve(name).await.is_err());
    }
}
