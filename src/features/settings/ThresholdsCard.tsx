import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { call, errorMessage } from "@/lib/ipc";
import { DEFAULT_DISK, DEFAULT_PRESSURE, useSettings } from "@/lib/settings";

function NumberField({ label, value, onChange }: { label: string; value: string; onChange: (v: string) => void }) {
  return (
    <label className="flex flex-col gap-1 text-xs text-muted-foreground">
      {label}
      <input
        inputMode="decimal"
        className="h-8 w-24 rounded-md border border-input bg-background px-2 text-sm text-foreground"
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
    </label>
  );
}

/** Editable alert thresholds. Rust validates them; the UI only shows what Rust says. */
export function ThresholdsCard() {
  const { settings, update } = useSettings();
  const p = settings.pressureThresholds;
  const d = settings.diskThresholds;
  const [ram, setRam] = useState([String(p.moderate), String(p.high), String(p.critical)]);
  const [disk, setDisk] = useState([String(d.warningFree), String(d.highFree), String(d.criticalFree)]);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  async function apply() {
    setError(null);
    setSaved(false);
    const [m, h, c] = ram.map(Number);
    const [w, hf, cf] = disk.map(Number);
    if ([m, h, c, w, hf, cf].some((n) => !Number.isFinite(n))) {
      setError("Every threshold must be a number.");
      return;
    }
    try {
      const rp = await call<{ moderate: number; high: number; critical: number }>("set_pressure_thresholds", { moderate: m, high: h, critical: c });
      const dp = await call<{ warningFree: number; highFree: number; criticalFree: number }>("set_disk_thresholds", { warningFree: w, highFree: hf, criticalFree: cf });
      update({ pressureThresholds: rp, diskThresholds: dp });
      setSaved(true);
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  function reset() {
    setRam([String(DEFAULT_PRESSURE.moderate), String(DEFAULT_PRESSURE.high), String(DEFAULT_PRESSURE.critical)]);
    setDisk([String(DEFAULT_DISK.warningFree), String(DEFAULT_DISK.highFree), String(DEFAULT_DISK.criticalFree)]);
    setSaved(false);
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Alert thresholds</CardTitle>
        <CardDescription>RAM states start at these usage percentages. Drive states start when free space falls below these percentages. Press Apply to save.</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <fieldset className="flex flex-wrap gap-3">
          <legend className="mb-1 text-xs font-medium">RAM used (%)</legend>
          <NumberField label="Moderate" value={ram[0]} onChange={(v) => setRam([v, ram[1], ram[2]])} />
          <NumberField label="High" value={ram[1]} onChange={(v) => setRam([ram[0], v, ram[2]])} />
          <NumberField label="Critical" value={ram[2]} onChange={(v) => setRam([ram[0], ram[1], v])} />
        </fieldset>
        <fieldset className="flex flex-wrap gap-3">
          <legend className="mb-1 text-xs font-medium">Drive free space (%)</legend>
          <NumberField label="Warning below" value={disk[0]} onChange={(v) => setDisk([v, disk[1], disk[2]])} />
          <NumberField label="High below" value={disk[1]} onChange={(v) => setDisk([disk[0], v, disk[2]])} />
          <NumberField label="Critical below" value={disk[2]} onChange={(v) => setDisk([disk[0], disk[1], v])} />
        </fieldset>
        {error && (
          <p role="alert" className="rounded-md border border-danger/40 bg-danger/10 p-2 text-sm">
            {error}
          </p>
        )}
        {saved && <p role="status" className="text-sm text-ok">Thresholds applied.</p>}
        <div className="flex gap-2">
          <Button size="sm" onClick={apply}>
            Apply
          </Button>
          <Button size="sm" variant="outline" onClick={reset}>
            Reset to defaults
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
