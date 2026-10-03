import { describe, expect, it } from "vitest";
import {
  liveSocketUrl,
  modifierBits,
  mouseButtonName,
  takeoverHref,
  toPagePoint,
} from "../browser-live";

describe("toPagePoint", () => {
  const page = { width: 1000, height: 500 };

  it("maps a point on an exactly-fitting frame", () => {
    const box = { left: 0, top: 0, width: 500, height: 250 };
    expect(toPagePoint(250, 125, box, page)).toEqual({ x: 500, y: 250 });
  });

  it("accounts for letterboxing above and below the frame", () => {
    // 500x500 box; a 2:1 page draws 500x250, centred with 125px bars.
    const box = { left: 10, top: 20, width: 500, height: 500 };
    expect(toPagePoint(10, 145, box, page)).toEqual({ x: 0, y: 0 });
    expect(toPagePoint(510, 395, box, page)).toEqual({ x: 1000, y: 500 });
  });

  it("ignores clicks in the letterbox", () => {
    const box = { left: 0, top: 0, width: 500, height: 500 };
    expect(toPagePoint(250, 50, box, page)).toBeNull();
  });

  it("returns null before the first frame has a size", () => {
    const box = { left: 0, top: 0, width: 500, height: 500 };
    expect(toPagePoint(10, 10, box, { width: 0, height: 0 })).toBeNull();
  });
});

describe("modifierBits", () => {
  it("uses CDP's bit layout", () => {
    expect(modifierBits({ altKey: true, ctrlKey: false, metaKey: false, shiftKey: false })).toBe(1);
    expect(modifierBits({ altKey: false, ctrlKey: true, metaKey: false, shiftKey: true })).toBe(10);
    expect(modifierBits({ altKey: false, ctrlKey: false, metaKey: true, shiftKey: false })).toBe(4);
  });
});

describe("mouseButtonName", () => {
  it("names the standard buttons", () => {
    expect(mouseButtonName(0)).toBe("left");
    expect(mouseButtonName(2)).toBe("right");
    expect(mouseButtonName(9)).toBe("none");
  });
});

describe("liveSocketUrl", () => {
  it("uses wss on an https origin, as behind a TLS reverse proxy", () => {
    expect(liveSocketUrl("google", "t0k", "https://frona.example.com")).toBe(
      "wss://frona.example.com/api/browser/live/ws?profile=google&token=t0k",
    );
  });

  it("uses ws on a plain http origin", () => {
    expect(liveSocketUrl("default", "x", "http://localhost:3001")).toBe(
      "ws://localhost:3001/api/browser/live/ws?profile=default&token=x",
    );
  });

  it("encodes the profile and token", () => {
    expect(liveSocketUrl("a b", "x/y+z", "https://h")).toBe(
      "wss://h/api/browser/live/ws?profile=a+b&token=x%2Fy%2Bz",
    );
  });
});

describe("takeoverHref", () => {
  it("keeps only the path and profile of an absolute link", () => {
    expect(takeoverHref("https://public.example/browser?profile=google")).toBe(
      "/browser?profile=google",
    );
  });

  it("opens the current profile for a link without one", () => {
    expect(takeoverHref("http://localhost:3001/browser")).toBe("/browser");
  });

  it("sends old debugger-proxy links to the live view", () => {
    expect(takeoverHref("/api/browser/debugger/cred-123")).toBe("/browser");
  });
});
