import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { pickView } from "../index";
import { MemoryGraphSparqlView } from "../memory-graph-sparql";
import { SafeToolView } from "../safe-tool-view";
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

vi.mock("@/components/ui/code-block", () => ({
  CodeBlock: ({ code, language, wrap }: { code: string; language?: string; wrap?: boolean }) => (
    <pre data-testid="code-block" data-lang={language ?? ""} data-wrap={wrap ? "1" : "0"}>
      {code}
    </pre>
  ),
}));

const QUERY = "SELECT ?person ?type WHERE { ?person a schema:Person; a ?type }";

describe("MemoryGraphSparqlView", () => {
  it("is registered for memory_graph_sparql", () => {
    expect(pickView("memory_graph_sparql")).toBe(MemoryGraphSparqlView);
  });

  it("renders rows in the backend's column order, marking unbound cells", () => {
    render(
      <MemoryGraphSparqlView
        {...mkProps({
          toolName: "memory_graph_sparql",
          args: { query: QUERY },
          result: JSON.stringify({
            kind: "solutions",
            columns: ["person", "type"],
            rows: [
              { person: "people/mina", type: "schema:Person" },
              { person: "people/sarah" },
            ],
            truncated: false,
          }),
        })}
      />,
    );

    expect(screen.getByText("Memory Graph Query")).toBeInTheDocument();
    expect(screen.getByText(/2 rows/)).toBeInTheDocument();
    expect(screen.getByTestId("code-block")).toHaveAttribute("data-lang", "sparql");
    expect(screen.getByTestId("code-block")).toHaveTextContent(QUERY);
    expect(screen.getAllByRole("columnheader").map((cell) => cell.textContent)).toEqual([
      "?person",
      "?type",
    ]);
    expect(screen.getByText("people/mina")).toBeInTheDocument();
    expect(screen.getByText("schema:Person")).toBeInTheDocument();
    expect(screen.getByText("unbound")).toBeInTheDocument();
  });

  it("renders an ASK result without a table", () => {
    render(
      <MemoryGraphSparqlView
        {...mkProps({
          toolName: "memory_graph_sparql",
          args: { query: "ASK { <urn:frona:kb:people/mina> a schema:Person }" },
          result: { kind: "boolean", value: true },
        })}
      />,
    );

    expect(screen.getByText(/ASK true/)).toBeInTheDocument();
    expect(screen.getByText("true")).toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
  });

  it("marks a truncated result set and says rows were omitted", () => {
    render(
      <MemoryGraphSparqlView
        {...mkProps({
          toolName: "memory_graph_sparql",
          args: { query: QUERY },
          result: {
            kind: "solutions",
            columns: ["person"],
            rows: [{ person: "people/mina" }],
            truncated: true,
          },
        })}
      />,
    );

    expect(screen.getByText(/1\+ rows/)).toBeInTheDocument();
    expect(screen.getByText("More rows not shown.")).toBeInTheDocument();
  });

  it("says so when a query matched nothing", () => {
    render(
      <MemoryGraphSparqlView
        {...mkProps({
          toolName: "memory_graph_sparql",
          args: { query: QUERY },
          result: { kind: "solutions", columns: ["person"], rows: [], truncated: false },
        })}
      />,
    );

    expect(screen.getByText(/0 rows/)).toBeInTheDocument();
    expect(screen.getByText("No rows.")).toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
  });

  it("hands an unknown result shape to the generic view", () => {
    render(
      <SafeToolView
        view={MemoryGraphSparqlView}
        {...mkProps({
          toolName: "memory_graph_sparql",
          args: { query: QUERY },
          result: "not-json",
        })}
      />,
    );

    expect(screen.getByText("Result:")).toBeInTheDocument();
    expect(screen.getByText("not-json")).toBeInTheDocument();
  });

  it("keeps the query and the error text on a failed call", () => {
    render(
      <SafeToolView
        view={MemoryGraphSparqlView}
        {...mkProps({
          toolName: "memory_graph_sparql",
          args: { query: QUERY },
          result: "Unknown prefix 'schema:'",
          status: { type: "incomplete", reason: "error", error: "Unknown prefix 'schema:'" },
        })}
      />,
    );

    expect(screen.getByTestId("code-block")).toHaveAttribute("data-lang", "sparql");
    expect(screen.getByText("SPARQL query failed")).toBeInTheDocument();
    expect(screen.getAllByText(/Unknown prefix/).length).toBeGreaterThan(0);
    expect(screen.queryByText("Result:")).not.toBeInTheDocument();
  });
});
