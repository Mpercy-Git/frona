use std::str::FromStr;

use cedar_policy::PolicySet;

use crate::core::error::AppError;
use crate::tool::ToolDefinition;

use super::schema::entity_type_name;

const VALID_ACTIONS: &[&str] = &[
    "invoke_tool", "delegate_task", "send_message",
    "read", "write", "connect", "bind",
];

pub fn validate_syntax(policy_text: &str) -> Result<(), AppError> {
    PolicySet::from_str(policy_text)
        .map_err(|e| AppError::Validation(format!("Invalid policy syntax: {e}")))?;
    Ok(())
}

pub fn validate_entities(
    policy_text: &str,
    tool_groups: &[String],
    tool_definitions: &[ToolDefinition],
) -> Result<Vec<String>, AppError> {
    validate_syntax(policy_text)?;

    let policy_set = PolicySet::from_str(policy_text)
        .map_err(|e| AppError::Validation(format!("Invalid policy syntax: {e}")))?;

    let tool_group_type = entity_type_name("ToolGroup");
    let tool_type = entity_type_name("Tool");
    let action_type = entity_type_name("Action");
    let path_type = entity_type_name("Path");
    let network_dest_type = entity_type_name("NetworkDestination");

    let tool_ids: Vec<&str> = tool_definitions.iter().map(|d| d.id.as_str()).collect();
    let mut warnings = Vec::new();

    for policy in policy_set.policies() {
        match policy.resource_constraint() {
            cedar_policy::ResourceConstraint::In(ref uid)
            | cedar_policy::ResourceConstraint::Eq(ref uid) => {
                if uid.type_name() == &tool_group_type
                    && !tool_groups.iter().any(|g| g == uid.id().unescaped())
                {
                    warnings.push(format!(
                        "ToolGroup '{}' does not exist. Available groups: {}",
                        uid.id().unescaped(), tool_groups.join(", ")
                    ));
                }
                if uid.type_name() == &tool_type
                    && !tool_ids.contains(&uid.id().unescaped())
                {
                    warnings.push(format!(
                        "Tool '{}' does not exist",
                        uid.id().unescaped()
                    ));
                }
                if uid.type_name() == &path_type {
                    let raw = uid.id().unescaped();
                    if !is_valid_policy_path(raw) {
                        warnings.push(format!(
                            "Path '{raw}' must be an absolute path (start with /) or a virtual path (user://name/... or agent://id/...)"
                        ));
                    }
                }
                if uid.type_name() == &network_dest_type {
                    let dest = uid.id().unescaped().to_string();
                    if !is_valid_network_destination(&dest) {
                        warnings.push(format!(
                            "NetworkDestination '{dest}' is not a valid IP, CIDR, hostname, or hostname:port"
                        ));
                    }
                }
            }
            _ => {}
        }

        if let cedar_policy::ActionConstraint::Eq(ref uid) = policy.action_constraint()
            && uid.type_name() == &action_type
            && !VALID_ACTIONS.contains(&uid.id().unescaped())
        {
            warnings.push(format!(
                "Action '{}' is not valid. Valid actions: {}",
                uid.id().unescaped(), VALID_ACTIONS.join(", ")
            ));
        }
    }

    Ok(warnings)
}

pub fn is_valid_policy_path(raw: &str) -> bool {
    if raw.starts_with('/') {
        return true;
    }
    crate::storage::path::VirtualPath::parse(raw).is_ok()
}

fn is_valid_network_destination(dest: &str) -> bool {
    if dest.is_empty() {
        return false;
    }

    // CIDR: 10.0.0.0/8 or 10.0.0.0/8!443
    if dest.contains('/') {
        let cidr_part = dest.split('!').next().unwrap_or(dest);
        let mut parts = cidr_part.splitn(2, '/');
        let ip = parts.next().unwrap_or_default();
        let prefix = parts.next().unwrap_or_default();
        return ip.parse::<std::net::IpAddr>().is_ok()
            && prefix.parse::<u8>().is_ok();
    }

    // Plain IP
    if dest.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }

    // Bracketed IPv6: [::1] or [::1]:443
    if dest.starts_with('[') {
        let inner = dest.trim_start_matches('[').split(']').next().unwrap_or_default();
        return inner.parse::<std::net::Ipv6Addr>().is_ok();
    }

    // Hostname or hostname:port
    let host = dest.split(':').next().unwrap_or(dest);
    host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && !host.starts_with('-')
        && !host.starts_with('.')
        && !host.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_network_ipv4() {
        assert!(is_valid_network_destination("1.2.3.4"));
        assert!(is_valid_network_destination("192.168.0.1"));
    }

    #[test]
    fn test_valid_network_ipv6() {
        assert!(is_valid_network_destination("::1"));
        assert!(is_valid_network_destination("[::1]"));
        assert!(is_valid_network_destination("[::1]:443"));
    }

    #[test]
    fn test_valid_network_cidr() {
        assert!(is_valid_network_destination("10.0.0.0/8"));
        assert!(is_valid_network_destination("192.168.0.0/16"));
        assert!(is_valid_network_destination("10.0.0.0/8!443"));
    }

    #[test]
    fn test_valid_network_hostname() {
        assert!(is_valid_network_destination("gmail.com"));
        assert!(is_valid_network_destination("api.example.com"));
        assert!(is_valid_network_destination("my-host.internal"));
    }

    #[test]
    fn test_valid_network_hostname_with_port() {
        assert!(is_valid_network_destination("gmail.com:443"));
        assert!(is_valid_network_destination("api.example.com:8080"));
    }

    #[test]
    fn test_invalid_network_empty() {
        assert!(!is_valid_network_destination(""));
    }

    #[test]
    fn test_invalid_network_bad_chars() {
        assert!(!is_valid_network_destination("host name"));
        assert!(!is_valid_network_destination("host/name"));
        assert!(!is_valid_network_destination("-leading-dash"));
        assert!(!is_valid_network_destination(".leading-dot"));
    }

    #[test]
    fn test_invalid_network_bad_cidr() {
        assert!(!is_valid_network_destination("not-ip/8"));
        assert!(!is_valid_network_destination("10.0.0.0/abc"));
    }

    #[test]
    fn test_valid_directory() {
        let result = validate_syntax(
            r#"permit(principal, action == Policy::Action::"read", resource == Policy::Path::"/data");"#,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_syntax() {
        let result = validate_syntax("not valid cedar");
        assert!(result.is_err());
    }
}
