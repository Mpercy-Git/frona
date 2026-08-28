import { describe, it, expect, vi, beforeEach } from "vitest";
import { useState } from "react";
import { render, screen, fireEvent } from "@testing-library/react";
import { ModelSelector } from "../model-selector";
import { getProviderModels } from "@/lib/config-types";

vi.mock("@/lib/config-types", () => ({
  getProviderModels: vi.fn(),
}));

const mockedGetProviderModels = vi.mocked(getProviderModels);

// The module-level model cache in model-selector is keyed by provider id, so
// every test uses its own provider to start from a clean fetch.
let providerSeq = 0;

function Harness({ initialModel = "" }: { initialModel?: string }) {
  const provider = `test-provider-${providerSeq}`;
  const [model, setModel] = useState(initialModel);
  return (
    <ModelSelector
      label="Main Model"
      provider={provider}
      model={model}
      enabledProviders={[provider]}
      onProviderChange={() => {}}
      onModelChange={setModel}
    />
  );
}

async function renderSelector(initialModel = "") {
  providerSeq += 1;
  render(<Harness initialModel={initialModel} />);
  // Wait for the model list to arrive — the label drops "(loading...)".
  return (await screen.findByRole("combobox", { name: "Model" })) as HTMLInputElement;
}

beforeEach(() => {
  mockedGetProviderModels.mockReset();
  mockedGetProviderModels.mockResolvedValue({
    models: [
      { id: "gpt-4o", name: "GPT-4o" },
      { id: "gpt-4o-mini", name: "GPT-4o mini" },
    ],
  });
});

describe("ModelSelector model field", () => {
  it("shows the friendly name for a model id that is in the list", async () => {
    const input = await renderSelector("gpt-4o-mini");
    expect(input.value).toBe("GPT-4o mini");
  });

  it("stays empty when the field is cleared", async () => {
    const input = await renderSelector("gpt-4o");
    fireEvent.change(input, { target: { value: "" } });
    expect(input.value).toBe("");
  });

  it("keeps partially typed text instead of replacing it with the first match", async () => {
    const input = await renderSelector();
    fireEvent.change(input, { target: { value: "gpt" } });
    expect(input.value).toBe("gpt");
  });

  it("keeps a free-text model id that no listed model matches", async () => {
    const input = await renderSelector();
    fireEvent.change(input, { target: { value: "some-unlisted-model" } });
    expect(input.value).toBe("some-unlisted-model");
  });

  it("reports model info only for an exact id match", async () => {
    const onModelInfo = vi.fn();
    providerSeq += 1;
    const provider = `test-provider-${providerSeq}`;
    const { rerender } = render(
      <ModelSelector
        label="Main Model"
        provider={provider}
        model="gpt"
        enabledProviders={[provider]}
        onProviderChange={() => {}}
        onModelChange={() => {}}
        onModelInfo={onModelInfo}
      />,
    );
    await screen.findByRole("combobox", { name: "Model" });
    expect(onModelInfo).toHaveBeenCalledWith(null);

    onModelInfo.mockClear();
    rerender(
      <ModelSelector
        label="Main Model"
        provider={provider}
        model="gpt-4o"
        enabledProviders={[provider]}
        onProviderChange={() => {}}
        onModelChange={() => {}}
        onModelInfo={onModelInfo}
      />,
    );
    expect(onModelInfo).toHaveBeenCalledWith({ id: "gpt-4o", name: "GPT-4o" });
  });
});
