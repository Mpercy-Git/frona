"use client";

import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Containers to try, best first. Chrome, Firefox and Edge record Opus in WebM;
 * Safari only records AAC in MP4. The extension is what the server keys the
 * attachment's content type off (`.weba` → `audio/webm`, `.m4a` → `audio/mp4`),
 * so it has to match the container or the recording would be filed as video.
 */
const FORMATS: { mimeType: string; extension: string }[] = [
  { mimeType: "audio/webm;codecs=opus", extension: "weba" },
  { mimeType: "audio/webm", extension: "weba" },
  { mimeType: "audio/mp4", extension: "m4a" },
  { mimeType: "audio/ogg;codecs=opus", extension: "ogg" },
];

/** Recording only works in a secure context (HTTPS or localhost) with a mic API. */
export function voiceRecordingSupported(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof MediaRecorder !== "undefined" &&
    !!navigator.mediaDevices?.getUserMedia
  );
}

export function pickRecordingFormat(
  isTypeSupported: (mimeType: string) => boolean = (t) => MediaRecorder.isTypeSupported(t),
): { mimeType: string; extension: string } | null {
  return FORMATS.find((f) => isTypeSupported(f.mimeType)) ?? null;
}

/** `voice-note-2026-10-05-1432.weba`: sortable, and unique to the minute. */
export function voiceNoteFilename(extension: string, now: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  const stamp = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}`;
  return `voice-note-${stamp}.${extension}`;
}

export type VoiceRecorderState = "idle" | "starting" | "recording";

/**
 * Microphone capture for voice notes. `start` asks for the mic and begins
 * recording; `stop` resolves with the finished recording as a `File`, ready to
 * attach; `cancel` throws the recording away. The mic is released as soon as
 * recording ends, so the browser's "in use" indicator doesn't linger.
 */
export function useVoiceRecorder(onError?: (message: string) => void) {
  const [state, setState] = useState<VoiceRecorderState>("idle");
  const [elapsedMs, setElapsedMs] = useState(0);
  const recorderRef = useRef<MediaRecorder | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const startedAtRef = useRef(0);
  const formatRef = useRef<{ mimeType: string; extension: string } | null>(null);

  const release = useCallback(() => {
    streamRef.current?.getTracks().forEach((t) => t.stop());
    streamRef.current = null;
    recorderRef.current = null;
    chunksRef.current = [];
    setElapsedMs(0);
    setState("idle");
  }, []);

  // Never leave the mic open if the composer unmounts mid-recording.
  useEffect(() => release, [release]);

  useEffect(() => {
    if (state !== "recording") return;
    const id = window.setInterval(() => setElapsedMs(Date.now() - startedAtRef.current), 250);
    return () => window.clearInterval(id);
  }, [state]);

  const start = useCallback(async () => {
    if (state !== "idle") return;
    if (!voiceRecordingSupported()) {
      onError?.("Voice recording needs a browser with microphone access over HTTPS.");
      return;
    }
    const format = pickRecordingFormat();
    if (!format) {
      onError?.("This browser can't record audio in a format Frona understands.");
      return;
    }
    setState("starting");
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const recorder = new MediaRecorder(stream, { mimeType: format.mimeType });
      chunksRef.current = [];
      recorder.ondataavailable = (e) => {
        if (e.data.size > 0) chunksRef.current.push(e.data);
      };
      streamRef.current = stream;
      recorderRef.current = recorder;
      formatRef.current = format;
      recorder.start(1000);
      startedAtRef.current = Date.now();
      setElapsedMs(0);
      setState("recording");
    } catch (e) {
      release();
      const denied = e instanceof DOMException && (e.name === "NotAllowedError" || e.name === "SecurityError");
      onError?.(
        denied
          ? "Microphone access was blocked. Allow it in your browser's site settings to record voice notes."
          : `Couldn't start recording: ${e instanceof Error ? e.message : String(e)}`,
      );
    }
  }, [state, onError, release]);

  const stop = useCallback((): Promise<File | null> => {
    const recorder = recorderRef.current;
    const format = formatRef.current;
    if (!recorder || !format || recorder.state === "inactive") {
      release();
      return Promise.resolve(null);
    }
    return new Promise((resolve) => {
      recorder.onstop = () => {
        const blob = new Blob(chunksRef.current, { type: format.mimeType.split(";")[0] });
        release();
        if (blob.size === 0) {
          resolve(null);
          return;
        }
        resolve(new File([blob], voiceNoteFilename(format.extension), { type: blob.type }));
      };
      recorder.stop();
    });
  }, [release]);

  const cancel = useCallback(() => {
    const recorder = recorderRef.current;
    if (recorder && recorder.state !== "inactive") {
      recorder.onstop = null;
      recorder.stop();
    }
    release();
  }, [release]);

  return { state, elapsedMs, start, stop, cancel };
}

export function formatElapsed(ms: number): string {
  const total = Math.floor(ms / 1000);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}
