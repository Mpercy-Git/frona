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

// The format the backend emits since page text was inlined: three fixed lines per
// hit, then a `<page>` block. Reading the path off the last line of the block used
// to pick up `</page>` and fold the prose into the description.
const RESULT_WITH_PAGES = `Top matches, page text included — call read(paths=[…]) only for a page whose text is cut off or not shown below:

- Mina  [person]
  Mina is a software engineer.
  /app/data/users/mina/pkm/Memory/people/me.md
<page path="/app/data/users/mina/pkm/Memory/people/me.md">
Mina lives in Atherton and works on Frona.
</page>

- Football  [Topic]
  Mina follows football.
  /app/data/users/mina/pkm/Memory/topic/football.md
  (no page text — frontmatter only; read it if you need the attributes)`;

describe("MemorySearchView", () => {
  it("reads the path and description off their own lines when page text is inlined", () => {
    render(
      <MemorySearchView
        {...mkProps({
          args: { query: "mina" },
          result: RESULT_WITH_PAGES,
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
    expect(screen.getByText(/2 matches/)).toBeInTheDocument();
  });

  it("shows the inlined page text, and copes with a hit that has none", () => {
    render(
      <MemorySearchView
        {...mkProps({
          args: { query: "mina" },
          result: RESULT_WITH_PAGES,
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
