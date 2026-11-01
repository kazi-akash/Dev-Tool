import { useState } from "react";
import { Monitor, Moon, Sun, type LucideIcon } from "lucide-react";
import { call, errorMessage } from "@/lib/ipc";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { useAsync } from "@/hooks/useAsync";
import type { AutostartStatus } from "@/types/system";
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
  const [testMsg, setTestMsg] = useState<string | null>(null);
  const startup = useAsync(() => call<AutostartStatus>("get_start_with_windows"));
  const [startupError, setStartupError] = useState<string | null>(null);

  async function changeStartup(enabled: boolean) {
    setStartupError(null);
    try {
      await call<AutostartStatus>("set_start_with_windows", { enabled });
    } catch (e) {
      setStartupError(errorMessage(e));
    }
    // Always re-read: the registry, not the click, is the truth.
    await startup.reload();
  }

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

      <Card>
        <CardHeader>
          <CardTitle>Notifications</CardTitle>
          <CardDescription>Windows toasts for the things worth interrupting you for.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <label className="flex items-center justify-between gap-3 text-sm">
            <span className="font-medium">Show desktop notifications</span>
            <Switch checked={settings.notifications} onCheckedChange={(v) => update({ notifications: v })} aria-label="Show desktop notifications" />
          </label>
          <p className="text-xs text-muted-foreground">
            You are told when RAM or a drive becomes critical, when memory is climbing fast, when a performance incident is recorded, and when a scan or action you started finishes while the window is hidden. A condition is announced once, not repeatedly; only a worse level interrupts it.
          </p>
          <Button size="sm" variant="outline" disabled={!settings.notifications} onClick={() => void call("send_test_notification").catch((e) => setTestMsg(errorMessage(e)))}>
            Send a test notification
          </Button>
          {testMsg && (
            <p role="alert" className="text-xs text-danger">
              {testMsg}
            </p>
          )}
        </CardContent>
      </Card>

      <AiCard />

      <Card>
        <CardHeader>
          <CardTitle>Background behavior</CardTitle>
          <CardDescription>Closing the window hides DevPilot to the system tray so monitoring keeps running. Use the tray menu to quit.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <label className="flex items-center justify-between gap-3 text-sm">
            <span className="font-medium">Start DevPilot with Windows</span>
            <Switch checked={startup.data?.enabled ?? false} disabled={!startup.data || startup.loading} onCheckedChange={(v) => void changeStartup(v)} aria-label="Start DevPilot with Windows" />
          </label>
          <p className="text-xs text-muted-foreground">
            Off unless you turn it on. When on, DevPilot starts in the system tray when you sign in: monitoring, incident recording and notifications run, and the window does not open. This only affects your Windows account and needs no administrator rights.
          </p>
          {startup.data?.enabled && !startup.data.pointsToThisApp && (
            <p role="status" className="text-xs">
              Windows is set to start a different copy of DevPilot ({startup.data.command}).{" "}
              <button type="button" className="underline" onClick={() => void changeStartup(true)}>
                Use this copy instead
              </button>
            </p>
          )}
          {startupError && (
            <p role="alert" className="text-xs text-danger">
              {startupError}
            </p>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
