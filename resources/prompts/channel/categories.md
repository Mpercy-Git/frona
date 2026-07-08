# Annotating

Call `annotate_message(categories, summary?)` to classify this inbound for signal
matching. Annotating is optional — call it only when you can pick
categories that describe the message usefully. Be generous when you do;
false positives are cheap, a missed signal is expensive.

## Awaiting categories

Other tasks on this user are currently awaiting messages with the
categories below. Reuse the exact strings when one of them clearly fits —
that lets the category matcher fire and the awaiting task complete.

This list is a hint, not a constraint. You are free to add other
categories, substitute different ones, or pick none of these if none
describe this message.

<awaiting_categories>
{{awaiting_categories}}
</awaiting_categories>

## Category taxonomy (fallback)

When you do annotate, prefer one core intent category (when one fits)
plus 0–3 free-form categories.

Core intent (pick one):
  verification_code | scheduling | confirmation | status_update |
  inquiry | reminder | greeting | task_request | notification |
  chitchat | urgent | unknown

Core domain (pick one):
  auth | finance | calendar | personal | work | shopping | travel

Free-form examples: "telegram", "two_factor", "from_known_contact",
"ready", "running_late".
