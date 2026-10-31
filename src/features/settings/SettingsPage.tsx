import { Monitor, Moon, Sun, type LucideIcon } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { useSettings, type ThemeMode } from "@/lib/settings";

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
            <Button
              key={value}
              variant={settings.theme === value ? "default" : "outline"}
              size="sm"
              aria-pressed={settings.theme === value}
              onClick={() => update({ theme: value })}
            >
              <Icon />
              {label}
            </Button>
          ))}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Monitoring interval</CardTitle>
          <CardDescription>
            How often system metrics are sampled. Applied once the monitor is added (Phase 2).
          </CardDescription>
        </CardHeader>
        <CardContent className="flex gap-2">
          {INTERVALS.map((sec) => (
            <Button
              key={sec}
              variant={settings.monitorIntervalSec === sec ? "default" : "outline"}
              size="sm"
              aria-pressed={settings.monitorIntervalSec === sec}
              onClick={() => update({ monitorIntervalSec: sec })}
            >
              {sec}s
            </Button>
          ))}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Background behavior</CardTitle>
          <CardDescription>
            Closing the window hides DevPilot to the system tray so monitoring keeps running. Use the tray menu to quit.
          </CardDescription>
        </CardHeader>
      </Card>
    </div>
  );
}
