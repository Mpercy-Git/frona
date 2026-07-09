"use client";

import { createContext, useContext, useCallback, useRef, useState } from "react";

interface SectionHandlers {
  save: () => Promise<void>;
  discard: () => void;
  hideDiscard?: boolean;
}

interface SettingsContextValue {
  /** Sections call this to report whether they have unsaved changes */
  setModified: (key: string, modified: boolean) => void;
  /** Sections call this to register save/discard handlers */
  register: (key: string, handlers: SectionHandlers) => void;
  /** Sections call this to unregister on unmount */
  unregister: (key: string) => void;
  /** Reload config from the server */
  refresh: () => Promise<void>;
}

const SettingsContext = createContext<SettingsContextValue | null>(null);

const noopSettings: SettingsContextValue = {
  setModified: () => {},
  register: () => {},
  unregister: () => {},
  refresh: () => Promise.resolve(),
};

export function useSettings() {
  const ctx = useContext(SettingsContext);
  return ctx ?? noopSettings;
}

interface SettingsProviderProps {
  children: React.ReactNode;
  onRefresh: () => Promise<void>;
  onModifiedChange: (anyModified: boolean) => void;
  onHandlersChange: (handlers: Map<string, SectionHandlers>) => void;
}

export function SettingsProvider({ children, onRefresh, onModifiedChange, onHandlersChange }: SettingsProviderProps) {
  const modifiedRef = useRef<Map<string, boolean>>(new Map());
  const handlersRef = useRef<Map<string, SectionHandlers>>(new Map());

  const setModified = useCallback((key: string, modified: boolean) => {
    modifiedRef.current.set(key, modified);
    const anyModified = Array.from(modifiedRef.current.values()).some(Boolean);
    onModifiedChange(anyModified);
  }, [onModifiedChange]);

  const register = useCallback((key: string, handlers: SectionHandlers) => {
    handlersRef.current.set(key, handlers);
    onHandlersChange(new Map(handlersRef.current));
  }, [onHandlersChange]);

  const unregister = useCallback((key: string) => {
    handlersRef.current.delete(key);
    modifiedRef.current.delete(key);
    onHandlersChange(new Map(handlersRef.current));
    const anyModified = Array.from(modifiedRef.current.values()).some(Boolean);
    onModifiedChange(anyModified);
  }, [onModifiedChange, onHandlersChange]);

  return (
    <SettingsContext.Provider value={{ setModified, register, unregister, refresh: onRefresh }}>
      {children}
    </SettingsContext.Provider>
  );
}

export type { SectionHandlers };
