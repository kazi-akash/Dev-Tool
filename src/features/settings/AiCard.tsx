import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { useAsync } from "@/hooks/useAsync";
import { call, errorMessage } from "@/lib/ipc";
import { useSettings } from "@/lib/settings";
import type { AiPayload, ProviderKind } from "@/types/api";

const PRESETS: { id: string; label: string; kind: ProviderKind; baseUrl: string; model: string }[] = [
  { id: "openai", label: "OpenAI", kind: "openAi", baseUrl: "https://api.openai.com/v1", model: "gpt-4o-mini" },
  { id: "anthropic", label: "Anthropic", kind: "anthropic", baseUrl: "https://api.anthropic.com", model: "claude-sonnet-5-5" },
  { id: "ollama", label: "Ollama (local)", kind: "openAi", baseUrl: "http://localhost:11434/v1", model: "llama3.1" },
];

const inputCls = "h-8 w-full rounded-md border border-input bg-background px-2 text-sm text-foreground";

export function AiCard() {
  const { settings, update } = useSettings();
  const ai = settings.ai;
  const keyStatus = useAsync(() => call<boolean>("ai_key_status"));
  const [key, setKey] = useState("");
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [payload, setPayload] = useState<AiPayload | null>(null);

  const set = (patch: Partial<typeof ai>) => update({ ai: { ...ai, ...patch } });

  async function saveKey() {
    try {
      await call("set_ai_key", { key });
      setKey("");
      setMsg({ ok: true, text: "API key saved in the Windows Credential Manager." });
      void keyStatus.reload();
    } catch (e) {
      setMsg({ ok: false, text: errorMessage(e) });
    }
  }

  async function removeKey() {
    try {
      await call("clear_ai_key");
      setMsg({ ok: true, text: "API key removed." });
      void keyStatus.reload();
    } catch (e) {
      setMsg({ ok: false, text: errorMessage(e) });
    }
  }

  async function showPayload() {
    try {
      setPayload(await call<AiPayload>("preview_ai_payload"));
      setMsg(null);
    } catch (e) {
      setMsg({ ok: false, text: errorMessage(e) });
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>AI assistance (optional)</CardTitle>
        <CardDescription>
          DevPilot works fully without AI. When enabled, a short, sanitized summary (numbers and process names, never files, paths, command lines or secrets) is sent to the provider you choose
          when you press “Ask AI”. The AI can only suggest; DevPilot never runs anything it says.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <label className="flex items-center justify-between gap-3 text-sm">
          <span className="font-medium">Enable AI assistance</span>
          <Switch checked={ai.enabled} onCheckedChange={(v) => set({ enabled: v })} aria-label="Enable AI assistance" />
        </label>

        <div className="flex flex-wrap gap-1" role="group" aria-label="Provider presets">
          {PRESETS.map((p) => (
            <Button key={p.id} size="sm" variant={ai.baseUrl === p.baseUrl && ai.kind === p.kind ? "default" : "outline"} onClick={() => set({ kind: p.kind, baseUrl: p.baseUrl, model: p.model })}>
              {p.label}
            </Button>
          ))}
        </div>

        <div className="grid gap-3 sm:grid-cols-2">
          <label className="text-xs text-muted-foreground">
            Base URL (https only; http only for localhost)
            <input className={inputCls} value={ai.baseUrl} onChange={(e) => set({ baseUrl: e.target.value })} spellCheck={false} />
          </label>
          <label className="text-xs text-muted-foreground">
            Model
            <input className={inputCls} value={ai.model} onChange={(e) => set({ model: e.target.value })} spellCheck={false} />
          </label>
          <label className="text-xs text-muted-foreground">
            API format
            <select className={inputCls} value={ai.kind} onChange={(e) => set({ kind: e.target.value as ProviderKind })}>
              <option value="openAi">OpenAI-compatible</option>
              <option value="anthropic">Anthropic-compatible</option>
            </select>
          </label>
        </div>

        <div>
          <label htmlFor="ai-api-key" className="text-xs text-muted-foreground">
            API key {keyStatus.data ? "(a key is stored)" : "(none stored)"}
          </label>
          <div className="mt-1 flex gap-2">
            <input id="ai-api-key" className={inputCls} type="password" autoComplete="off" placeholder="Paste a key to store it" value={key} onChange={(e) => setKey(e.target.value)} />
            <Button size="sm" disabled={!key.trim()} onClick={saveKey}>
              Save
            </Button>
            <Button size="sm" variant="outline" disabled={!keyStatus.data} onClick={removeKey}>
              Remove
            </Button>
          </div>
          <p className="mt-1 text-xs text-muted-foreground">Stored in the Windows Credential Manager. It is never written to a file or shown again. Local servers such as Ollama need no key.</p>
        </div>

        {msg && (
          <p role={msg.ok ? "status" : "alert"} className={`rounded-md border p-2 text-sm ${msg.ok ? "border-ok/40 bg-ok/10" : "border-danger/40 bg-danger/10"}`}>
            {msg.text}
          </p>
        )}

        <div>
          <Button size="sm" variant="outline" onClick={showPayload}>
            Show exactly what would be sent
          </Button>
          {payload && (
            <pre className="mt-2 max-h-64 overflow-auto rounded-md border bg-muted/40 p-2 text-xs" aria-label="AI payload preview">
              {JSON.stringify(payload, null, 2)}
            </pre>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
