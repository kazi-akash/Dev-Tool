import { useState } from "react";
import { Link } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { call, errorMessage } from "@/lib/ipc";
import { useSettings } from "@/lib/settings";
import type { AiPayload } from "@/types/api";

/**
 * Optional second opinion. The answer is shown as plain text; it is never parsed, rendered as HTML,
 * or passed to the action engine. Actions only ever come from the rule engine's recommendations.
 */
export function AiAssistant() {
  const { settings } = useSettings();
  const { ai } = settings;
  const [question, setQuestion] = useState("");
  const [answer, setAnswer] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [payload, setPayload] = useState<AiPayload | null>(null);

  async function ask() {
    setBusy(true);
    setError(null);
    try {
      const r = await call<{ text: string }>("ask_ai", {
        enabled: ai.enabled,
        config: { kind: ai.kind, baseUrl: ai.baseUrl, model: ai.model },
        question: question.trim() || null,
      });
      setAnswer(r.text);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  async function preview() {
    try {
      setPayload(await call<AiPayload>("preview_ai_payload"));
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  if (!ai.enabled) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>AI assistant</CardTitle>
        </CardHeader>
        <CardContent className="text-sm text-muted-foreground">
          Off. The diagnosis above is produced locally and needs no AI. You can turn on an optional assistant in{" "}
          <Link to="/settings" className="text-primary hover:underline">
            Settings
          </Link>
          ; nothing is sent until you press “Ask AI”.
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>AI assistant</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-xs text-muted-foreground">
          Sends a summary (numbers and process names only) to <span className="font-medium text-foreground">{new URL(ai.baseUrl, "https://x").host || ai.baseUrl}</span>. The AI can suggest, not act.
        </p>
        <textarea
          className="h-16 w-full rounded-md border border-input bg-background p-2 text-sm"
          placeholder="Optional question, for example: why does my PC freeze when I build?"
          value={question}
          maxLength={500}
          onChange={(e) => setQuestion(e.target.value)}
          aria-label="Question for the AI"
        />
        <div className="flex gap-2">
          <Button size="sm" onClick={ask} disabled={busy}>
            {busy ? "Asking..." : "Ask AI"}
          </Button>
          <Button size="sm" variant="outline" onClick={preview}>
            Show what will be sent
          </Button>
        </div>
        {error && (
          <p role="alert" className="rounded-md border border-danger/40 bg-danger/10 p-2 text-sm">
            {error}
          </p>
        )}
        {answer && (
          <div aria-label="AI answer" className="whitespace-pre-wrap rounded-md border bg-muted/40 p-3 text-sm">
            {answer}
          </div>
        )}
        {payload && (
          <pre className="max-h-64 overflow-auto rounded-md border bg-muted/40 p-2 text-xs" aria-label="AI payload preview">
            {JSON.stringify(payload, null, 2)}
          </pre>
        )}
      </CardContent>
    </Card>
  );
}
