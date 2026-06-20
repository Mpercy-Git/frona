import { describe, it, expect } from "vitest";
import { bestSeverity, groupEvents, parseShellOutput } from "../shell-sandbox";

const SAMPLE = `{"id":"abc","sid":"nifty_ellis","syd":266,"ctx":"access","cap":"net/connect","act":"deny","sys":"connect","addr":"104.20.23.154!80","ipv":4,"time":"20260614T020918Z","cmd":"curl example.com","cwd":"/app","pid":267,"uid":1000,"tip":"configure \`allow/net/connect+104.20.23.154!80'"}
{"id":"abc","sid":"nifty_ellis","syd":266,"ctx":"access","cap":"net/connect","act":"deny","sys":"connect","addr":"172.66.147.243!80","ipv":4,"time":"20260614T020918Z","cmd":"curl example.com","cwd":"/app","pid":267,"uid":1000,"tip":"configure \`allow/net/connect+172.66.147.243!80'"}
{"id":"abc","sid":"nifty_ellis","syd":266,"ctx":"access","cap":"net/connect","act":"deny","sys":"connect","addr":"104.20.23.154!80","ipv":4,"time":"20260614T020918Z","cmd":"curl example.com","cwd":"/app","pid":263,"uid":1000,"tip":"configure \`allow/net/connect+104.20.23.154!80'"}
{"id":"abc","sid":"nifty_ellis","syd":266,"ctx":"access","cap":"net/connect","act":"deny","sys":"connect","addr":"172.66.147.243!80","ipv":4,"time":"20260614T020918Z","cmd":"curl example.com","cwd":"/app","pid":263,"uid":1000,"tip":"configure \`allow/net/connect+172.66.147.243!80'"}`;

describe("parseShellOutput — sandbox event detection", () => {
  it("returns empty for empty input", () => {
    const r = parseShellOutput("");
    expect(r.events).toEqual([]);
    expect(r.remainingText).toBe("");
  });

  it("extracts and dedupes the 4-line curl-blocked sample to 2 events", () => {
    const r = parseShellOutput(SAMPLE);
    expect(r.events).toHaveLength(2);
    expect(r.remainingText).toBe("");
    expect(r.events.map((e) => e.target)).toEqual([
      "104.20.23.154:80",
      "172.66.147.243:80",
    ]);
    expect(r.events[0]).toMatchObject({
      cap: "net/connect",
      act: "deny",
      severity: "high",
      tip: expect.stringContaining("allow/net/connect+104.20.23.154"),
    });
  });

  it("converts the `!` host/port separator to `:` for net addresses", () => {
    const r = parseShellOutput(
      '{"ctx":"access","cap":"net/bind","act":"deny","sys":"bind","addr":"0.0.0.0!8080"}',
    );
    expect(r.events[0].target).toBe("0.0.0.0:8080");
  });

  it("uses `path` as target for non-network capabilities", () => {
    const r = parseShellOutput(
      '{"ctx":"access","cap":"file/read","act":"deny","sys":"openat","path":"/etc/shadow"}',
    );
    expect(r.events[0].target).toBe("/etc/shadow");
  });

  it("falls back to argv[0] then cmd then sys when no addr/path present", () => {
    const a = parseShellOutput(
      '{"ctx":"access","cap":"exec","act":"deny","sys":"execve","argv":["/bin/sh","-c","x"]}',
    );
    expect(a.events[0].target).toBe("/bin/sh");

    const b = parseShellOutput(
      '{"ctx":"access","cap":"exec","act":"deny","sys":"execve","cmd":"curl x"}',
    );
    expect(b.events[0].target).toBe("curl x");

    const c = parseShellOutput(
      '{"ctx":"access","cap":"ioctl","act":"deny","sys":"ioctl"}',
    );
    expect(c.events[0].target).toBe("ioctl");
  });

  it("ignores allow/warn events — only deny-class actions surface", () => {
    const text = [
      '{"ctx":"access","cap":"net/connect","act":"allow","sys":"connect","addr":"1.1.1.1!80"}',
      '{"ctx":"access","cap":"net/connect","act":"warn","sys":"connect","addr":"1.1.1.2!80"}',
      '{"ctx":"access","cap":"net/connect","act":"deny","sys":"connect","addr":"1.1.1.3!80"}',
    ].join("\n");
    const r = parseShellOutput(text);
    expect(r.events).toHaveLength(1);
    expect(r.events[0].target).toBe("1.1.1.3:80");
  });

  it("strips a syd event that's glued to the end of a previous line (curl \\r progress case)", () => {
    // Curl writes its progress bar with \r (no newline), so when stderr is
    // captured to a buffer the trailing chars of the progress line are
    // directly concatenated with the next stderr write — the sandbox JSON.
    const text =
      '  0     0    0     0    0     0      0      0 --:--:-- --:--:--     0{"ctx":"access","cap":"net/connect","act":"deny","sys":"connect","addr":"1.2.3.4!80","tip":"hint"}\nnext line';
    const r = parseShellOutput(text);
    expect(r.events).toHaveLength(1);
    expect(r.events[0].target).toBe("1.2.3.4:80");
    expect(r.remainingText).toBe(
      "  0     0    0     0    0     0      0      0 --:--:-- --:--:--     0\nnext line",
    );
  });

  it("collapses runs of blank lines left after stripping consecutive events", () => {
    const text = [
      "header",
      '{"ctx":"access","cap":"net/connect","act":"deny","sys":"connect","addr":"1!80"}',
      '{"ctx":"access","cap":"net/connect","act":"deny","sys":"connect","addr":"2!80"}',
      '{"ctx":"access","cap":"net/connect","act":"deny","sys":"connect","addr":"3!80"}',
      "",
      "footer",
    ].join("\n");
    const r = parseShellOutput(text);
    expect(r.events).toHaveLength(3);
    // Three blank lines (from the stripped events) + the existing blank →
    // collapsed to a single blank line.
    expect(r.remainingText).toBe("header\n\nfooter");
  });

  it("keeps non-syd JSON in the output verbatim", () => {
    const text = 'prefix {"hello":"world"} suffix';
    const r = parseShellOutput(text);
    expect(r.events).toEqual([]);
    expect(r.remainingText).toBe('prefix {"hello":"world"} suffix');
  });

  it("strips allow/warn syd events from remainingText too (no JSON leak)", () => {
    const text = [
      "normal line",
      '{"ctx":"access","cap":"net/connect","act":"allow","sys":"connect","addr":"1.1.1.1!80"}',
      '{"ctx":"access","cap":"net/connect","act":"warn","sys":"connect","addr":"1.1.1.2!80"}',
      "another line",
    ].join("\n");
    const r = parseShellOutput(text);
    expect(r.events).toEqual([]);
    // Stripped events leave the surrounding newlines; consecutive runs
    // collapse to a single blank line via the 3+→2 newline rule.
    expect(r.remainingText).toBe("normal line\n\nanother line");
  });

  it("assigns severity by act", () => {
    const text = [
      '{"ctx":"access","cap":"net","act":"filter","sys":"x","addr":"1!80"}',
      '{"ctx":"access","cap":"net","act":"deny","sys":"x","addr":"2!80"}',
      '{"ctx":"access","cap":"net","act":"kill","sys":"x","addr":"3!80"}',
      '{"ctx":"access","cap":"net","act":"panic","sys":"x","addr":"4!80"}',
    ].join("\n");
    const r = parseShellOutput(text);
    expect(r.events.map((e) => e.severity)).toEqual([
      "low",
      "high",
      "critical",
      "critical",
    ]);
  });

  it("passes through non-JSON lines verbatim in remainingText", () => {
    const text = [
      "stderr:",
      '  % Total    % Received % Xferd',
      '{"ctx":"access","cap":"net/connect","act":"deny","sys":"connect","addr":"1.1.1.1!80"}',
      "  0     0    0     0",
    ].join("\n");
    const r = parseShellOutput(text);
    expect(r.events).toHaveLength(1);
    // A single blank line marks where the stripped event was (the surrounding
    // `\n`s survive — only the JSON itself is removed).
    expect(r.remainingText).toBe(
      "stderr:\n  % Total    % Received % Xferd\n\n  0     0    0     0",
    );
  });

  it("ignores malformed JSON", () => {
    const text = [
      '{"ctx":"access","cap":"x","act":"deny",sys broken}',
      "{not json at all}",
      "normal text",
    ].join("\n");
    const r = parseShellOutput(text);
    expect(r.events).toEqual([]);
    expect(r.remainingText.split("\n")).toHaveLength(3);
  });

  it("ignores JSON without the required ctx/act/sys/cap fields", () => {
    const text = [
      '{"hello":"world"}',
      '{"ctx":"x","act":"deny"}',
      '{"ctx":"x","act":"deny","sys":"y"}',
    ].join("\n");
    const r = parseShellOutput(text);
    expect(r.events).toEqual([]);
  });

  it("preserves the tip from the first event when later dupes lack one", () => {
    const text = [
      '{"ctx":"access","cap":"net/connect","act":"deny","sys":"connect","addr":"1!80","tip":"do X"}',
      '{"ctx":"access","cap":"net/connect","act":"deny","sys":"connect","addr":"1!80"}',
    ].join("\n");
    const r = parseShellOutput(text);
    expect(r.events).toHaveLength(1);
    expect(r.events[0].tip).toBe("do X");
  });
});

describe("bestSeverity", () => {
  it("returns null for empty list", () => {
    expect(bestSeverity([])).toBeNull();
  });

  it("returns critical when any event is critical", () => {
    const r = parseShellOutput(
      [
        '{"ctx":"access","cap":"x","act":"filter","sys":"x","addr":"1!80"}',
        '{"ctx":"access","cap":"x","act":"deny","sys":"x","addr":"2!80"}',
        '{"ctx":"access","cap":"x","act":"kill","sys":"x","addr":"3!80"}',
      ].join("\n"),
    );
    expect(bestSeverity(r.events)).toBe("critical");
  });

  it("returns high when no critical but any high", () => {
    const r = parseShellOutput(
      [
        '{"ctx":"access","cap":"x","act":"filter","sys":"x","addr":"1!80"}',
        '{"ctx":"access","cap":"x","act":"deny","sys":"x","addr":"2!80"}',
      ].join("\n"),
    );
    expect(bestSeverity(r.events)).toBe("high");
  });

  it("returns low when only filter events", () => {
    const r = parseShellOutput(
      '{"ctx":"access","cap":"x","act":"filter","sys":"x","addr":"1!80"}',
    );
    expect(bestSeverity(r.events)).toBe("low");
  });
});

describe("groupEvents", () => {
  it("groups by cap+act", () => {
    const r = parseShellOutput(SAMPLE);
    const groups = groupEvents(r.events);
    expect(groups.size).toBe(1);
    const [key, list] = Array.from(groups.entries())[0];
    expect(key).toBe("net/connect|deny");
    expect(list).toHaveLength(2);
  });

  it("separates events with different cap or act into distinct groups", () => {
    const text = [
      '{"ctx":"access","cap":"net/connect","act":"deny","sys":"x","addr":"1!80"}',
      '{"ctx":"access","cap":"file/read","act":"deny","sys":"x","path":"/etc/shadow"}',
      '{"ctx":"access","cap":"net/connect","act":"kill","sys":"x","addr":"2!80"}',
    ].join("\n");
    const r = parseShellOutput(text);
    const groups = groupEvents(r.events);
    expect(groups.size).toBe(3);
  });
});
