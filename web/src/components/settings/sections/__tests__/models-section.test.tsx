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

function Harness({ initial }: { initial: Record<string, ModelGroupConfig> }) {
  const [models, setModels] = useState(initial);
  return (
    <ModelsSection
      models={models}
      enabledProviders={["openai"]}
      onChange={(v) => {
        lastChange = v;
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
});

describe("ModelsSection group names", () => {
  it("keeps the capitalisation the user typed", () => {
    render(<Harness initial={{ primary: group() }} />);
    expandGroup("Primary");
    renameTo("MyGroup");

    expect(Object.keys(lastChange).filter((k) => lastChange[k] != null)).toEqual(["MyGroup"]);
  });

  it("turns spaces and invalid characters into a usable id without lower-casing", () => {
    render(<Harness initial={{ primary: group() }} />);
    expandGroup("Primary");
    renameTo("Fast Draft!");

    expect(Object.keys(lastChange).filter((k) => lastChange[k] != null)).toEqual(["Fast_Draft"]);
  });

  it("still resolves a predefined group's label back to its canonical id", () => {
    render(<Harness initial={{ group_1: group() }} />);
    expandGroup("group_1");
    renameTo("Coding");

    expect(Object.keys(lastChange).filter((k) => lastChange[k] != null)).toEqual(["coding"]);
  });

  it("marks the old name for deletion so the rename survives a save", () => {
    render(<Harness initial={{ primary: group() }} />);
    expandGroup("Primary");
    renameTo("MyGroup");

    // The backend deep-merges this patch, so the old key has to be sent as null
    // to be dropped from the config on disk.
    expect(lastChange.primary).toBeNull();
    expect(lastChange.MyGroup).toEqual(group());
  });

  it("gives a new group a usable default id", () => {
    render(<Harness initial={{ primary: group() }} />);
    fireEvent.click(screen.getByRole("button", { name: "+ Add Model Group" }));

    expect(Object.keys(lastChange)).toContain("group_1");
    expect(Object.keys(lastChange)).not.toContain("");
  });

  it("does not reuse the id of an existing group", () => {
    render(<Harness initial={{ group_1: group() }} />);
    fireEvent.click(screen.getByRole("button", { name: "+ Add Model Group" }));

    expect(Object.keys(lastChange).sort()).toEqual(["group_1", "group_2"]);
    expect(lastChange.group_1).toEqual(group());
  });
});
