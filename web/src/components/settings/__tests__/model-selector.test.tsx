import { describe, it, expect, vi } from "vitest";
import { useState } from "react";
import { render, screen, fireEvent } from "@testing-library/react";
import { ModelSelector } from "../model-selector";
import type { ModelDirectory, ProviderModelRow } from "@/lib/provider-admin";
import type { ModelProviderConfig } from "@/lib/config-types";

function row(id: string, name?: string): ProviderModelRow {
  return {
    id, name: name ?? null, description: null, context_window: null, max_tokens: null,
    suggested_protocol: "responses", protocols: [], availability: "account", configured_in: [],
    capabilities: { reasoning: null, tool_call: null, structured_output: null, input: [], output: [] },
    sources: [], warnings: [],
  };
}

function directory(rows: ProviderModelRow[]): ModelDirectory {
  return {
    connection: "openai", credential_method: "api_key", access_mode: "api", source: "openai",
    directory_status: "live", source_status: {}, manual_entry: true, models: rows,
  };
}

function Harness({ initialModel = "", initialProvider = "openai", enabledProviders = ["openai"], providerConfigs = {}, directory: dir, loading }: {
  initialModel?: string; initialProvider?: string; enabledProviders?: string[];
  providerConfigs?: Record<string, ModelProviderConfig>; directory?: ModelDirectory; loading?: boolean;
}) {
  const [provider, setProvider] = useState(initialProvider);
  const [model, setModel] = useState(initialModel);
  return (
    <ModelSelector
      provider={provider}
      model={model}
      enabledProviders={enabledProviders}
      providerConfigs={providerConfigs}
      directory={dir}
      loading={loading}
      onProviderChange={setProvider}
      onModelChange={setModel}
    />
  );
}

describe("ModelSelector provider field", () => {
  it("shows a brand label for a known provider", () => {
    render(<Harness enabledProviders={["openai", "acme"]} providerConfigs={{ acme: { provider: "acme", enabled: true, api_key: null, base_url: null } }} />);
    expect(screen.getByRole("combobox", { name: "Provider" })).toHaveValue("OpenAI");
  });

  it("falls back to a formatted handle for an unknown provider brand", () => {
    render(<Harness initialProvider="acme" enabledProviders={["acme"]} providerConfigs={{ acme: { provider: "acme", enabled: true, api_key: null, base_url: null } }} />);
    expect(screen.getByRole("combobox", { name: "Provider" })).toHaveValue("Acme");
  });

  it("shows the connection handle alongside the brand when they differ", () => {
    render(
      <Harness
        initialProvider="my-account"
        enabledProviders={["my-account"]}
        providerConfigs={{ "my-account": { provider: "openrouter", enabled: true, api_key: null, base_url: null } }}
      />,
    );
    expect(screen.getByRole("combobox", { name: "Provider" })).toHaveValue("OpenRouter (my-account)");
  });

  it("marks a selected provider that is no longer enabled as disabled and ignores a click on it", () => {
    const onProviderChange = vi.fn();
    render(
      <ModelSelector
        provider="openai"
        model=""
        enabledProviders={[]}
        providerConfigs={{}}
        onProviderChange={onProviderChange}
        onModelChange={() => {}}
      />,
    );
    fireEvent.keyDown(screen.getByRole("combobox", { name: "Provider" }), { key: "ArrowDown" });
    fireEvent.click(screen.getByRole("option", { name: "OpenAI (disabled)" }));
    expect(onProviderChange).not.toHaveBeenCalled();
  });

  it("only switches the selected provider when it is enabled", () => {
    render(<Harness enabledProviders={["openai", "anthropic"]} />);
    fireEvent.keyDown(screen.getByRole("combobox", { name: "Provider" }), { key: "ArrowDown" });
    fireEvent.click(screen.getByRole("option", { name: "Anthropic" }));
    expect(screen.getByRole("combobox", { name: "Provider" })).toHaveValue("Anthropic");
  });
});

describe("ModelSelector model field", () => {
  it("disables the model field until a provider is selected", () => {
    render(<Harness initialProvider="" enabledProviders={["openai"]} />);
    expect(screen.getByRole("combobox", { name: "Model" })).toBeDisabled();
  });

  it("shows the friendly name for a model id that is in the directory", () => {
    render(<Harness initialModel="gpt-4o-mini" directory={directory([row("gpt-4o-mini", "GPT-4o mini")])} />);
    expect(screen.getByRole("combobox", { name: "Model" })).toHaveValue("GPT-4o mini");
  });

  it("commits a free-text model id on blur without waiting for an exact directory match", () => {
    render(<Harness directory={directory([row("gpt-4o")])} />);
    const input = screen.getByRole("combobox", { name: "Model" });
    fireEvent.change(input, { target: { value: "some-unlisted-model" } });
    expect(input).toHaveValue("some-unlisted-model");
    fireEvent.blur(input);
    expect(input).toHaveValue("some-unlisted-model");
  });

  it("commits a directory model as soon as the typed text matches its id, without waiting for blur", () => {
    const onModelChange = vi.fn();
    render(
      <ModelSelector provider="openai" model="" enabledProviders={["openai"]} providerConfigs={{}}
        directory={directory([row("gpt-4o", "GPT-4o"), row("gpt-4o-mini", "GPT-4o mini")])}
        onProviderChange={() => {}} onModelChange={onModelChange} />,
    );
    fireEvent.change(screen.getByRole("combobox", { name: "Model" }), { target: { value: "gpt-4o" } });
    expect(onModelChange).toHaveBeenCalledWith("gpt-4o");
  });

  it("shows a loading placeholder while the directory is being fetched", () => {
    render(<Harness loading />);
    expect(screen.getByRole("combobox", { name: "Model" })).toHaveAttribute("placeholder", "Fetching models...");
  });

  it("resets the draft to the committed model when the model prop changes externally", () => {
    const { rerender } = render(
      <ModelSelector provider="openai" model="gpt-4o" enabledProviders={["openai"]} providerConfigs={{}}
        onProviderChange={() => {}} onModelChange={() => {}} />,
    );
    const input = screen.getByRole("combobox", { name: "Model" }) as HTMLInputElement;
    expect(input.value).toBe("gpt-4o");
    rerender(
      <ModelSelector provider="openai" model="gpt-4o-mini" enabledProviders={["openai"]} providerConfigs={{}}
        onProviderChange={() => {}} onModelChange={() => {}} />,
    );
    expect(input.value).toBe("gpt-4o-mini");
  });
});
