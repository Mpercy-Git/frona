import { describe, expect, it } from "vitest";
import { activeSearchMatches, canvasResolution, neighborhoodByDepth, seededPosition, selectionCameraTarget, wikilinksToMarkdown, wrapLabel } from "../memory-graph";
import type { MemoryGraphEdge, MemorySearchResult } from "../memory-types";

const edges: MemoryGraphEdge[] = [
  { id: "ab", fromPath: "a", toPath: "b", relation: "knows", label: "knows", origin: "asserted", sourceMemoryIds: [] },
  { id: "bc", fromPath: "b", toPath: "c", relation: "worksFor", label: "works for", origin: "asserted", sourceMemoryIds: [] },
  { id: "dc", fromPath: "d", toPath: "c", relation: "owns", label: "owns", origin: "inferred", sourceMemoryIds: [] },
  { id: "de", fromPath: "d", toPath: "e", relation: "near", label: "near", origin: "asserted", sourceMemoryIds: [] },
];

describe("memory graph focus", () => {
  it("classifies an undirected two-hop neighborhood around the selected page", () => {
    const depths = neighborhoodByDepth("a", edges, 2);
    expect(Object.fromEntries(depths)).toEqual({ a: 0, b: 1, c: 2 });
    expect(depths.has("d")).toBe(false);
  });

  it("gives a page path the same initial position on every load", () => {
    expect(seededPosition("people/me")).toEqual(seededPosition("people/me"));
    expect(seededPosition("people/me")).not.toEqual(seededPosition("projects/frona"));
  });

  it("turns wiki links into navigable memory links", () => {
    expect(wikilinksToMarkdown("See [[people/alex|Alex]] and [[projects/orbit]].")).toBe(
      "See [Alex](memory:people%2Falex) and [projects/orbit](memory:projects%2Forbit).",
    );
  });

  it("stops masking the graph after a search result is selected", () => {
    const results = [{ path: "people/christine" }] as MemorySearchResult[];
    expect(activeSearchMatches(true, "christine", results)).toEqual(new Set(["people/christine"]));
    expect(activeSearchMatches(false, "christine", results)).toBeNull();
  });

  it("centers a selected node without zooming back out to the whole graph", () => {
    expect(selectionCameraTarget({ x: 0.2, y: 0.7 }, { x: 0.5, y: 0.5, ratio: 0.08, angle: 0 })).toEqual({
      x: 0.2,
      y: 0.7,
      ratio: 0.08,
    });
    expect(selectionCameraTarget({ x: 0.2, y: 0.7 }, { ratio: 1 })).toEqual({
      x: 0.2,
      y: 0.7,
      ratio: 0.35,
    });
  });

  it("keeps a Retina canvas at the same visible size as the graph viewport", () => {
    expect(canvasResolution(800, 600, 2)).toEqual({
      pixelWidth: 1600,
      pixelHeight: 1200,
      cssWidth: "800px",
      cssHeight: "600px",
    });
  });

  it("wraps graph labels after every three words", () => {
    expect(wrapLabel("Prepare and file a Form N-400 application")).toEqual([
      "Prepare and file",
      "a Form N-400",
      "application",
    ]);
  });
});
