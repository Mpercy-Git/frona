import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  formatElapsed,
  pickRecordingFormat,
  useVoiceRecorder,
  voiceNoteFilename,
} from "../use-voice-recorder";

class FakeRecorder {
  static supported = new Set(["audio/webm;codecs=opus", "audio/webm", "audio/mp4"]);
  static isTypeSupported(type: string) {
    return FakeRecorder.supported.has(type);
  }
  static last: FakeRecorder | null = null;

  state: "inactive" | "recording" = "inactive";
  ondataavailable: ((e: { data: Blob }) => void) | null = null;
  onstop: (() => void) | null = null;

  constructor(
    public stream: MediaStream,
    public options: { mimeType: string },
  ) {
    FakeRecorder.last = this;
  }
  start() {
    this.state = "recording";
  }
  stop() {
    this.state = "inactive";
    this.ondataavailable?.({ data: new Blob(["opus-bytes"], { type: this.options.mimeType }) });
    this.onstop?.();
  }
}

const stopTrack = vi.fn();
const getUserMedia = vi.fn();

beforeEach(() => {
  FakeRecorder.last = null;
  stopTrack.mockReset();
  getUserMedia.mockReset();
  getUserMedia.mockResolvedValue({ getTracks: () => [{ stop: stopTrack }] });
  vi.stubGlobal("MediaRecorder", FakeRecorder);
  Object.defineProperty(navigator, "mediaDevices", {
    configurable: true,
    value: { getUserMedia },
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("recording format", () => {
  it("prefers Opus in WebM, filed under the audio extension", () => {
    expect(pickRecordingFormat((t) => t.startsWith("audio/webm"))).toEqual({
      mimeType: "audio/webm;codecs=opus",
      extension: "weba",
    });
  });

  it("falls back to MP4 for Safari", () => {
    expect(pickRecordingFormat((t) => t === "audio/mp4")).toEqual({
      mimeType: "audio/mp4",
      extension: "m4a",
    });
  });

  it("reports nothing when no container is recordable", () => {
    expect(pickRecordingFormat(() => false)).toBeNull();
  });
});

describe("helpers", () => {
  it("names notes by local time", () => {
    expect(voiceNoteFilename("weba", new Date(2026, 9, 5, 9, 7))).toBe(
      "voice-note-2026-10-05-0907.weba",
    );
  });

  it("formats elapsed time as m:ss", () => {
    expect(formatElapsed(0)).toBe("0:00");
    expect(formatElapsed(65_400)).toBe("1:05");
  });
});

describe("useVoiceRecorder", () => {
  it("records, then hands back an audio file and releases the mic", async () => {
    const { result } = renderHook(() => useVoiceRecorder());

    await act(() => result.current.start());
    expect(result.current.state).toBe("recording");
    expect(getUserMedia).toHaveBeenCalledWith({ audio: true });
    expect(FakeRecorder.last?.options.mimeType).toBe("audio/webm;codecs=opus");

    let file: File | null = null;
    await act(async () => {
      file = await result.current.stop();
    });

    expect(file).not.toBeNull();
    expect(file!.type).toBe("audio/webm");
    expect(file!.name).toMatch(/^voice-note-.*\.weba$/);
    expect(stopTrack).toHaveBeenCalled();
    expect(result.current.state).toBe("idle");
  });

  it("cancel discards the recording and releases the mic", async () => {
    const { result } = renderHook(() => useVoiceRecorder());
    await act(() => result.current.start());

    act(() => result.current.cancel());

    expect(stopTrack).toHaveBeenCalled();
    expect(result.current.state).toBe("idle");
    let file: File | null = new File([], "x");
    await act(async () => {
      file = await result.current.stop();
    });
    expect(file).toBeNull();
  });

  it("explains a blocked microphone", async () => {
    getUserMedia.mockRejectedValue(new DOMException("denied", "NotAllowedError"));
    const onError = vi.fn();
    const { result } = renderHook(() => useVoiceRecorder(onError));

    await act(() => result.current.start());

    expect(result.current.state).toBe("idle");
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("Microphone access was blocked"));
  });

  it("says so when the browser can't record at all", async () => {
    vi.stubGlobal("MediaRecorder", undefined);
    const onError = vi.fn();
    const { result } = renderHook(() => useVoiceRecorder(onError));

    await act(() => result.current.start());

    expect(getUserMedia).not.toHaveBeenCalled();
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("microphone access over HTTPS"));
  });
});
