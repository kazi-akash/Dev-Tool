import { Monitor, Moon, Sun, type LucideIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { useSettings, type ThemeMode } from "@/lib/settings";
import { AiCard } from "./AiCard";
import { ThresholdsCard } from "./ThresholdsCard";

const THEMES: { value: ThemeMode; label: string; icon: LucideIcon }[] = [
  { value: "system", label: "System", icon: Monitor },
  { value: "light", label: "Light", icon: Sun },
  { value: "dark", label: "Dark", icon: Moon },
];

const INTERVALS = [1, 2, 5, 10];

export function SettingsPage() {
  const { settings, update } = useSettings();

  return (
    <div className="mx-auto max-w-3xl space-y-4">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">Settings</h1>
        <p className="mt-1 text-sm text-muted-foreground">Preferences are stored locally on this machine.</p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Appearance</CardTitle>
          <CardDescription>Choose a color theme.</CardDescription>
        </CardHeader>
        <CardContent className="flex gap-2">
          {THEMES.map(({ value, label, icon: Icon }) => (
            <Button key={value} variant={settings.theme === value ? "default" : "outline"} size="sm" aria-pressed={settings.theme === value} onClick={() => update({ theme: value })}>
              <Icon />
              {label}
            </Button>
          ))}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Monitoring interval</CardTitle>
          <CardDescription>How often system metrics are sampled.</CardDescription>
        </CardHeader>
        <CardContent className="flex gap-2">
          {INTERVALS.map((sec) => (
            <Button key={sec} variant={settings.monitorIntervalSec === sec ? "default" : "outline"} size="sm" aria-pressed={settings.monitorIntervalSec === sec} onClick={() => update({ monitorIntervalSec: sec })}>
              {sec}s
            </Button>
          ))}
        </CardContent>
      </Card>

      <ThresholdsCard />

      <Card>
        <CardHeader>
          <CardTitle>Safe automation</CardTitle>
          <CardDescription>Off by default. When on, DevPilot may clean temporary files older than 24 hours, at most once an hour, when a drive is running low.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <label className="flex items-center justify-between gap-3 text-sm">
            <span className="font-medium">Automatically perform safe optimizations</span>
            <Switch checked={settings.safeAutomation} onCheckedChange={(v) => update({ safeAutomation: v })} aria-label="Automatically perform safe optimizations" />
          </label>
          <p className="text-xs text-muted-foreground">
            It never deletes other files, closes programs, edits the registry or Windows files, uninstalls software, or touches your documents. Everything it does appears in Actions history.
          </p>
        </CardContent>
      </Card>

      <AiCard />

      <Card>
        <CardHeader>
          <CardTitle>Background behavior</CardTitle>
          <CardDescription>Closing the window hides DevPilot to the system tray so monitoring keeps running. Use the tray menu to quit.</CardDescription>
        </CardHeader>
      </Card>
    </div>
  );
}
