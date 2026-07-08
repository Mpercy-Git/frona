"use client";

import { useState, useRef, useEffect } from "react";
import { SectionHeader, SectionPanel } from "@/components/settings/field";
import { IdentificationIcon, TrashIcon, PlusIcon } from "@heroicons/react/24/outline";

interface Entry {
  id: number;
  key: string;
  value: string;
}

interface IdentitySectionProps {
  identity: Record<string, string>;
  onChange: (identity: Record<string, string>) => void;
}

function toEntries(identity: Record<string, string>): Entry[] {
  return Object.entries(identity).map(([key, value], i) => ({ id: i, key, value }));
}

function toRecord(entries: Entry[]): Record<string, string> {
  const rec: Record<string, string> = {};
  for (const e of entries) {
    if (e.key) rec[e.key] = e.value;
  }
  return rec;
}

export function IdentitySection({ identity, onChange }: IdentitySectionProps) {
  const [entries, setEntries] = useState<Entry[]>(() => toEntries(identity));
  const nextId = useRef(entries.length);
  const internalChange = useRef(false);

  // Sync from parent when identity changes externally
  useEffect(() => {
    if (internalChange.current) {
      internalChange.current = false;
      return;
    }
    setEntries(toEntries(identity));
  }, [identity]);

  const update = (updated: Entry[]) => {
    setEntries(updated);
    internalChange.current = true;
    onChange(toRecord(updated));
  };

  const updateKey = (id: number, newKey: string) => {
    update(entries.map((e) => (e.id === id ? { ...e, key: newKey } : e)));
  };

  const updateValue = (id: number, newValue: string) => {
    update(entries.map((e) => (e.id === id ? { ...e, value: newValue } : e)));
  };

  const remove = (id: number) => {
    update(entries.filter((e) => e.id !== id));
  };

  const add = () => {
    const id = nextId.current++;
    update([...entries, { id, key: "", value: "" }]);
  };

  return (
    <div>
      <SectionHeader title="Identity" description="Custom identity attributes for this agent" icon={IdentificationIcon} />
      <SectionPanel>
        {entries.length > 0 && (
          <div className="grid grid-cols-[1fr_2fr_auto] gap-x-2 gap-y-2 items-center">
            <span className="text-xs font-medium text-text-tertiary">Key</span>
            <span className="text-xs font-medium text-text-tertiary">Value</span>
            <span className="w-8" />
            {entries.map((entry) => (
              <Row key={entry.id} entry={entry} onKeyChange={updateKey} onValueChange={updateValue} onRemove={remove} />
            ))}
          </div>
        )}
        {entries.length === 0 && (
          <p className="text-sm text-text-tertiary">No identity attributes defined.</p>
        )}
        <button
          onClick={add}
          className="flex items-center gap-1 text-sm text-accent hover:underline"
        >
          <PlusIcon className="h-4 w-4" />
          Add attribute
        </button>
      </SectionPanel>
    </div>
  );
}

function Row({ entry, onKeyChange, onValueChange, onRemove }: {
  entry: Entry;
  onKeyChange: (id: number, key: string) => void;
  onValueChange: (id: number, value: string) => void;
  onRemove: (id: number) => void;
}) {
  return (
    <>
      <input
        value={entry.key}
        onChange={(e) => onKeyChange(entry.id, e.target.value)}
        placeholder="Key"
        className="rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
      />
      <input
        value={entry.value}
        onChange={(e) => onValueChange(entry.id, e.target.value)}
        placeholder="Value"
        className="rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent focus:outline-none"
      />
      <button
        onClick={() => onRemove(entry.id)}
        className="flex items-center justify-center w-8 h-8 rounded-lg text-text-tertiary hover:bg-surface-tertiary hover:text-text-primary transition"
      >
        <TrashIcon className="h-4 w-4" />
      </button>
    </>
  );
}
