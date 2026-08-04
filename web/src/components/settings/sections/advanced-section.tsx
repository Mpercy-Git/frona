"use client";

import type { InferenceConfig, SchedulerConfig, AppConfig } from "@/lib/config-types";
import { NumberInput, TextInput, Toggle, SectionHeader, SectionPanel } from "@/components/settings/field";

/** Parse a comma-separated list into trimmed, non-empty entries. */
function parseList(value: string): string[] {
  return value.split(",").map((s) => s.trim()).filter(Boolean);
}
import { AdjustmentsHorizontalIcon, CpuChipIcon, ClockIcon, Square3Stack3DIcon } from "@heroicons/react/24/outline";

interface AdvancedSectionProps {
  inference: InferenceConfig;
  scheduler: SchedulerConfig;
  app: AppConfig;
  onChange: (update: { inference?: InferenceConfig; scheduler?: SchedulerConfig; app?: AppConfig }) => void;
}

export function AdvancedSection({ inference, scheduler, app, onChange }: AdvancedSectionProps) {
  return (
    <div className="space-y-6">
      <SectionHeader title="Advanced" description="Inference, scheduling, and app hosting settings" icon={AdjustmentsHorizontalIcon} />

      <SectionPanel title="Inference" icon={CpuChipIcon}>

        <NumberInput
          label="Max Tool Turns"
          description="Maximum number of tool call turns per inference request"
          value={inference.max_tool_turns}
          onChange={(max_tool_turns) => onChange({ inference: { ...inference, max_tool_turns } })}
          min={1}
          placeholder="200"
        />

        <NumberInput
          label="Default Max Tokens"
          description="Default maximum tokens for LLM responses"
          value={inference.default_max_tokens}
          onChange={(default_max_tokens) => onChange({ inference: { ...inference, default_max_tokens } })}
          min={1}
          placeholder="8192"
        />

        <NumberInput
          label="Compaction Trigger (%)"
          description="Context window usage percentage that triggers compaction"
          value={inference.compaction_trigger_pct}
          onChange={(compaction_trigger_pct) => onChange({ inference: { ...inference, compaction_trigger_pct } })}
          min={1}
          max={100}
          placeholder="80"
        />

        <NumberInput
          label="History Truncation (%)"
          description="Context window usage percentage that triggers history truncation"
          value={inference.history_truncation_pct}
          onChange={(history_truncation_pct) => onChange({ inference: { ...inference, history_truncation_pct } })}
          min={1}
          max={100}
          placeholder="90"
        />

        <NumberInput
          label="Tool Timeout (seconds)"
          description="Per-tool-call execution timeout. A hung tool (e.g. an unresponsive MCP server) fails after this instead of stalling the message. 0 disables."
          value={inference.tool_timeout_secs}
          onChange={(tool_timeout_secs) => onChange({ inference: { ...inference, tool_timeout_secs } })}
          min={0}
          placeholder="600"
        />

        <TextInput
          label="Vision Models"
          description="Model ids to force as vision-capable (comma-separated), overriding the catalog. Matches a bare id, provider/model, or vendor-prefixed suffix."
          value={(inference.vision_models ?? []).join(", ")}
          onChange={(v) => onChange({ inference: { ...inference, vision_models: parseList(v) } })}
          placeholder="provider/model, ..."
        />

        <TextInput
          label="Text-Only Models"
          description="Model ids to force as text-only (comma-separated). Images sent to these are transcribed by a vision model, or stripped. Wins over the catalog and Vision Models."
          value={(inference.text_only_models ?? []).join(", ")}
          onChange={(v) => onChange({ inference: { ...inference, text_only_models: parseList(v) } })}
          placeholder="deepseek-v4-flash, ..."
        />

        <Toggle
          label="Transcribe When Vision Unknown"
          description="When a model's image support is unknown (absent from the catalog and both lists), treat it as text-only so images are transcribed or stripped instead of risking a provider 404."
          value={inference.transcribe_when_vision_unknown ?? false}
          onChange={(transcribe_when_vision_unknown) => onChange({ inference: { ...inference, transcribe_when_vision_unknown } })}
        />
      </SectionPanel>

      <SectionPanel title="Scheduler" icon={ClockIcon}>

        <NumberInput
          label="Space Compaction Interval (hours)"
          description="How often to run space memory compaction"
          value={Math.round(scheduler.space_compaction_secs / 3600)}
          onChange={(hours) => onChange({ scheduler: { ...scheduler, space_compaction_secs: hours * 3600 } })}
          min={1}
          placeholder="1"
        />

        <NumberInput
          label="Memory Compaction Interval (hours)"
          description="How often to run memory compaction"
          value={Math.round(scheduler.memory_compaction_secs / 3600)}
          onChange={(hours) => onChange({ scheduler: { ...scheduler, memory_compaction_secs: hours * 3600 } })}
          min={1}
          placeholder="1"
        />

        <NumberInput
          label="Poll Interval (seconds)"
          description="How often the scheduler checks for pending tasks"
          value={scheduler.poll_secs}
          onChange={(poll_secs) => onChange({ scheduler: { ...scheduler, poll_secs } })}
          min={1}
          placeholder="60"
        />
      </SectionPanel>

      <SectionPanel title="Apps" icon={Square3Stack3DIcon}>

        <NumberInput
          label="Port Range Start"
          description="First port in the range allocated to hosted apps"
          value={app.port_range_start}
          onChange={(port_range_start) => onChange({ app: { ...app, port_range_start } })}
          min={1024}
          max={65535}
          placeholder="4000"
        />

        <NumberInput
          label="Port Range End"
          description="Last port in the range allocated to hosted apps"
          value={app.port_range_end}
          onChange={(port_range_end) => onChange({ app: { ...app, port_range_end } })}
          min={1024}
          max={65535}
          placeholder="4100"
        />

        <NumberInput
          label="Health Check Timeout (seconds)"
          description="How long to wait for an app to respond to health checks"
          value={app.health_check_timeout_secs}
          onChange={(health_check_timeout_secs) => onChange({ app: { ...app, health_check_timeout_secs } })}
          min={1}
          placeholder="30"
        />

        <NumberInput
          label="Max Restart Attempts"
          description="Maximum number of automatic restart attempts for a crashed app"
          value={app.max_restart_attempts}
          onChange={(max_restart_attempts) => onChange({ app: { ...app, max_restart_attempts } })}
          min={0}
          placeholder="3"
        />

        <NumberInput
          label="Hibernate After (days)"
          description="Idle time before an app is automatically hibernated"
          value={Math.round(app.hibernate_after_secs / 86400)}
          onChange={(days) => onChange({ app: { ...app, hibernate_after_secs: days * 86400 } })}
          min={1}
          placeholder="3"
        />
      </SectionPanel>
    </div>
  );
}
