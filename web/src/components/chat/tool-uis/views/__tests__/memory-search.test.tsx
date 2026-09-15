import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemorySearchView } from "../memory-search";
import { mkProps } from "./helpers";

vi.mock("motion/react", () => ({
  AnimatePresence: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  motion: new Proxy({}, {
    get: () => (props: Record<string, unknown>) => {
      const { children, ...rest } = props as { children?: React.ReactNode } & Record<string, unknown>;
      const filtered: Record<string, unknown> = {};
      for (const [k, v] of Object.entries(rest)) {
        if (k === "initial" || k === "animate" || k === "exit" || k === "transition") continue;
        filtered[k] = v;
      }
      return <div {...filtered}>{children}</div>;
    },
  }),
}));

// Matches crates/frona-server/src/memory/pkm/tools.rs SearchTool output:
// a `read(<path>)` header and the absolute `.md` path of each hit.
const RESULT = `Top matches — read(<path>) to open one:

- Mina  [person]
  Mina is a software engineer at Amazon, originally from Egypt and living in Atherton, California.
  /app/data/users/mina/pkm/Memory/people/me.md

- Football  [Topic]
  Mina follows football (soccer) and is currently tracking the World Cup.
  /app/data/users/mina/pkm/Memory/topic/football.md`;

// The other half of the pairing with the backend. This file is emitted by
// crates/frona-server/src/memory/pkm/tools.rs and asserted there by
// `result_shape_matches_the_committed_fixture`; parsing it here is what stops the
// two drifting apart again. A backend format change fails the Rust test, and
// regenerating this file to match fails the tests below until the parser is taught
// the new shape - which is the step that was missing when page text was inlined.
const FIXTURE = readFileSync(
  resolve(
    dirname(fileURLToPath(import.meta.url)),
    "../../../../../../../resources/fixtures/memory_search_result.txt",
  ),
  "utf8",
);

describe("MemorySearchView", () => {
  it("reads the path and description off their own lines when page text is inlined", () => {
    render(
      <MemorySearchView
        {...mkProps({
          args: { query: "mina" },
          result: FIXTURE,
          isExpanded: true,
        })}
      />,
    );

    expect(
      screen.getByText("/app/data/users/mina/pkm/Memory/people/me.md"),
    ).toBeInTheDocument();
    expect(screen.getByText("Mina is a software engineer.")).toBeInTheDocument();
    // The page markup must never reach the rendered hit.
    expect(screen.queryByText(/<\/page>/)).not.toBeInTheDocument();
    expect(screen.getByText(/3 matches/)).toBeInTheDocument();
  });

  // Found by the fixture the moment it was shared with the backend: the third hit in
  // it is an untyped page, which the backend emits as `- Redis  []`. The parser's
  // regex required a non-empty tag, so that one hit failed the whole parse and the
  // view fell back to raw text for every result containing an unclassified page.
  it("parses a hit whose page has not been typed yet", () => {
    render(
      <MemorySearchView
        {...mkProps({ args: { query: "redis" }, result: FIXTURE, isExpanded: true })}
      />,
    );

    expect(screen.getByText("Redis")).toBeInTheDocument();
    expect(screen.getByText("the cache nobody has authored yet")).toBeInTheDocument();
    // The other hits still render - the failure mode was all-or-nothing.
    expect(screen.getByText("Mina")).toBeInTheDocument();
  });

  it("shows the inlined page text, and copes with a hit that has none", () => {
    render(
      <MemorySearchView
        {...mkProps({
          args: { query: "mina" },
          result: FIXTURE,
          isExpanded: true,
        })}
      />,
    );

    expect(
      screen.getByText("Mina lives in Atherton and works on Frona."),
    ).toBeInTheDocument();
    expect(
      screen.getByText("/app/data/users/mina/pkm/Memory/topic/football.md"),
    ).toBeInTheDocument();
  });

  it("renders Memory Search title and a query + count subtitle", () => {
    render(
      <MemorySearchView
        {...mkProps({ toolName: "memory_search", args: { query: "Mina" }, result: RESULT })}
      />,
    );
    expect(screen.getByText("Memory Search")).toBeInTheDocument();
    expect(screen.getByText(/Mina · 2 matches/)).toBeInTheDocument();
  });

  it("renders each hit with name, tag, description, and path", () => {
    render(
      <MemorySearchView
        {...mkProps({ toolName: "memory_search", args: { query: "Mina" }, result: RESULT })}
      />,
    );
    expect(screen.getByText("Mina")).toBeInTheDocument();
    expect(screen.getByText("person")).toBeInTheDocument();
    expect(screen.getByText(/software engineer at Amazon/)).toBeInTheDocument();
    expect(screen.getByText("/app/data/users/mina/pkm/Memory/people/me.md")).toBeInTheDocument();

    expect(screen.getByText("Football")).toBeInTheDocument();
    expect(screen.getByText("Topic")).toBeInTheDocument();
    expect(screen.getByText("/app/data/users/mina/pkm/Memory/topic/football.md")).toBeInTheDocument();
  });

  it('renders "No pages matched." for the empty case', () => {
    render(
      <MemorySearchView
        {...mkProps({
          toolName: "memory_search",
          args: { query: "x" },
          result: "No pages matched. The KB doesn't model this yet — ask the user, or reformulate.",
        })}
      />,
    );
    expect(screen.getByText("No pages matched.")).toBeInTheDocument();
  });

  it("falls back to raw <pre> output when the result doesn't match the expected format", () => {
    render(
      <MemorySearchView
        {...mkProps({ toolName: "memory_search", args: { query: "x" }, result: "Unparseable blob" })}
      />,
    );
    expect(screen.getByText("Unparseable blob")).toBeInTheDocument();
  });

  it("disables expansion when there's no result yet", () => {
    render(
      <MemorySearchView
        {...mkProps({ toolName: "memory_search", args: { query: "x" }, result: undefined })}
      />,
    );
    expect(screen.getByRole("button")).toBeDisabled();
  });

  it("lists every query of a batched search in the subtitle", () => {
    render(
      <MemorySearchView
        {...mkProps({
          toolName: "memory_search",
          args: { queries: ["postgres host", "postgres port"] },
          result: RESULT,
        })}
      />,
    );
    expect(screen.getByText(/postgres host · postgres port/)).toBeInTheDocument();
  });
});
