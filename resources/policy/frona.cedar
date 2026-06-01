@id("invoke-tool")
@description("Allow all agents to use all tools except restricted groups")
permit(
  principal,
  action == Policy::Action::"invoke_tool",
  resource
) unless { resource in Policy::ToolGroup::"agent" || resource in Policy::ToolGroup::"policy" };

@id("agent-management")
@description("Permit agent management tools when the caller is the system agent")
permit(
  principal,
  action == Policy::Action::"invoke_tool",
  resource in Policy::ToolGroup::"agent"
) when { principal.handle == "system" };

@id("policy-management")
@description("Permit policy management tools when the caller is the system agent")
permit(
  principal,
  action == Policy::Action::"invoke_tool",
  resource in Policy::ToolGroup::"policy"
) when { principal.handle == "system" };

@id("delegation")
@description("Allow agents to delegate tasks only to agents whose tools are a subset of their own")
permit(
  principal,
  action == Policy::Action::"delegate_task",
  resource
) when { principal.tools.containsAll(resource.tools) };

@id("communication")
@description("Allow all agents to send messages")
permit(
  principal,
  action == Policy::Action::"send_message",
  resource
);

@id("receive-signal")
@description("Allow all agents to receive signals from any source by default; operators add forbid policies for sensitive sources")
permit(
  principal,
  action == Policy::Action::"receive_signal",
  resource
);

@id("self-source")
@description("Allow inbound messages only from the channel's paired operator. Pairing is the access gate: an unpaired channel has no addresses and so denies everyone; a paired channel allows only the matching sender. Operators add additional permit policies to open up specific contacts or sources.")
permit(
  principal,
  action == Policy::Action::"receive_message",
  resource
)
when { context.paired_addresses.contains(resource.sender.address) };

@id("admin-user-management")
@description("Members of the 'admins' user group may list, modify, deactivate, and delete user accounts.")
permit(
  principal in Policy::UserGroup::"admins",
  action in [Policy::Action::"list_users", Policy::Action::"manage_users"],
  resource
);
