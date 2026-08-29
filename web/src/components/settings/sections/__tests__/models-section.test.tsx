import { describe, it, expect, vi, beforeEach } from "vitest";
import { useState } from "react";
import { render, screen, fireEvent } from "@testing-library/react";
import { ModelsSection } from "../models-section";
import type { ModelGroupConfig } from "@/lib/config-types";

vi.mock("@/lib/config-types", () => ({
  getProviderModels: vi.fn().mockResolvedValue({ models: [] }),
}));

function group(): ModelGroupConfig {
  return { provider: "openai", model: "gpt-4o", fallbacks: [] };
}

let lastChange: Record<string, ModelGroupConfig> = {};
let lastRemoved: string[] = [];

function Harness({ initial }: { initial: Record<string, ModelGroupConfig> }) {
  const [models, setModels] = useState(initial);
  return (
    <ModelsSection
      models={models}
      enabledProviders={["openai"]}
      onChange={(v, removed) => {
        lastChange = v;
        lastRemoved = removed ?? [];
        setModels(v);
      }}
    />
  );
}

function expandGroup(header: string) {
  fireEvent.click(screen.getByRole("button", { name: header }));
}

function renameTo(name: string) {
  const input = screen.getByRole("combobox", { name: "Group ID" });
  fireEvent.change(input, { target: { value: name } });
  fireEvent.blur(input);
}

beforeEach(() => {
  lastChange = {};
  lastRemoved = [];
});

describe("ModelsSection group names", () => {
  it("keeps the capitalisation the user typed", () => {
    render(<Harness initial={{ primary: group(), my_group: group() }} />);
    expandGroup("My Group");
    renameTo("MyGroup");

    expect(Object.keys(lastChange)).toContain("MyGroup");
  });

  it("turns spaces and invalid characters into a usable id without lower-casing", () => {
    render(<Harness initial={{ primary: group(), my_group: group() }} />);
    expandGroup("My Group");
    renameTo("Fast Draft!");

    expect(Object.keys(lastChange)).toContain("Fast_Draft");
  });

  it("still resolves a predefined group's label back to its canonical id", () => {
    render(<Harness initial={{ group_1: group() }} />);
    expandGroup("Group 1");
    renameTo("Coding");

    expect(Object.keys(lastChange)).toContain("coding");
  });

  it("reports the old name as removed so the rename survives a save", () => {
    render(<Harness initial={{ primary: group(), my_group: group() }} />);
    expandGroup("My Group");
    renameTo("MyGroup");

    // The old key is named in the removed list rather than tombstoned with a
    // null, so the backend drops it from the config on disk.
    expect(lastRemoved).toContain("my_group");
    expect(lastChange.MyGroup).toEqual(group());
    expect(lastChange.my_group).toBeUndefined();
  });

  it("gives a new group a usable default id", () => {
    render(<Harness initial={{ primary: group() }} />);
    fireEvent.click(screen.getByRole("button", { name: "+ Add Model Group" }));

    expect(Object.keys(lastChange)).toContain("custom_group");
    expect(Object.keys(lastChange)).not.toContain("");
  });

  it("does not reuse the id of an existing group", () => {
    render(<Harness initial={{ group_1: group() }} />);
    fireEvent.click(screen.getByRole("button", { name: "+ Add Model Group" }));

    expect(Object.keys(lastChange).sort()).toEqual(["custom_group", "group_1"]);
    expect(lastChange.group_1).toEqual(group());
  });
});
