import type { MemoryGraphEdge, MemorySearchResult } from "./memory-types";

export function activeSearchMatches(
  searchMode: boolean,
  query: string,
  results: MemorySearchResult[],
): Set<string> | null {
  if (!searchMode || !query.trim()) return null;
  return new Set(results.map((result) => result.path));
}

export function selectionCameraTarget(
  node: { x: number; y: number },
  camera: { x?: number; y?: number; ratio: number; angle?: number },
): { x: number; y: number; ratio: number } {
  return { x: node.x, y: node.y, ratio: Math.min(camera.ratio, 0.35) };
}

export function canvasResolution(width: number, height: number, pixelRatio: number) {
  return {
    pixelWidth: width * pixelRatio,
    pixelHeight: height * pixelRatio,
    cssWidth: `${width}px`,
    cssHeight: `${height}px`,
  };
}

export function wrapLabel(label: string, wordsPerLine = 3): string[] {
  const words = label.trim().split(/\s+/).filter(Boolean);
  const lines: string[] = [];
  for (let index = 0; index < words.length; index += wordsPerLine) {
    lines.push(words.slice(index, index + wordsPerLine).join(" "));
  }
  return lines;
}

export function neighborhoodByDepth(
  selectedPath: string,
  edges: MemoryGraphEdge[],
  maxDepth = 2,
): Map<string, number> {
  const adjacency = new Map<string, Set<string>>();
  for (const edge of edges) {
    if (!adjacency.has(edge.fromPath)) adjacency.set(edge.fromPath, new Set());
    if (!adjacency.has(edge.toPath)) adjacency.set(edge.toPath, new Set());
    adjacency.get(edge.fromPath)!.add(edge.toPath);
    adjacency.get(edge.toPath)!.add(edge.fromPath);
  }
  const depths = new Map<string, number>([[selectedPath, 0]]);
  const queue = [selectedPath];
  while (queue.length) {
    const path = queue.shift()!;
    const depth = depths.get(path)!;
    if (depth >= maxDepth) continue;
    for (const neighbor of adjacency.get(path) ?? []) {
      if (depths.has(neighbor)) continue;
      depths.set(neighbor, depth + 1);
      queue.push(neighbor);
    }
  }
  return depths;
}

function hashPath(path: string): number {
  let hash = 2166136261;
  for (let index = 0; index < path.length; index += 1) {
    hash ^= path.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

export function seededPosition(path: string): { x: number; y: number } {
  const first = hashPath(path);
  const second = hashPath(`${path}:y`);
  return {
    x: (first / 0xffffffff) * 2 - 1,
    y: (second / 0xffffffff) * 2 - 1,
  };
}

const BRANCH_COLORS = ["#648b72", "#667f9d", "#8a7697", "#9a7d5e", "#668c8b", "#8b746e", "#737d8c"];

export function branchColor(branch: string, depth = 0): string {
  const color = BRANCH_COLORS[hashPath(branch) % BRANCH_COLORS.length];
  if (depth === 0) return color;
  const amount = Math.min(depth, 4) * 7;
  const rgb = color.slice(1).match(/.{2}/g)!.map((part) => Number.parseInt(part, 16));
  return `rgb(${rgb.map((channel) => Math.min(255, channel + amount)).join(", ")})`;
}

export function wikilinksToMarkdown(source: string): string {
  return source.replace(/\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g, (_match, path: string, label?: string) => {
    const normalizedPath = path.trim();
    return `[${label?.trim() || normalizedPath}](memory:${encodeURIComponent(normalizedPath)})`;
  });
}
