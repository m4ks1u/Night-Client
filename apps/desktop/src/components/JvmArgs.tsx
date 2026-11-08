import { useEffect, useId, useState } from "react";
import { Button } from "./Button";
import { Card, InlineError } from "./ui";
import { api, toCmdError, type CmdError, type Instance } from "../lib/ipc";
import { useApp } from "../stores/app";

/** Render stored arguments back to editable text, quoting where needed. */
export function joinArgs(args: string[]): string {
  return args.map((a) => (a === "" || /[\s"]/.test(a) ? `"${a.replace(/"/g, '\\"')}"` : a)).join(" ");
}

export function JvmArgsCard({ instance, disabled }: { instance: Instance; disabled: boolean }) {
  const upsert = useApp((s) => s.upsertInstance);
  const saved = joinArgs(instance.jvmArgs);
  const [text, setText] = useState(saved);
  const [error, setError] = useState<CmdError | null>(null);
  const [busy, setBusy] = useState(false);
  const [ok, setOk] = useState(false);
  const id = useId();
  const hint = useId();

  useEffect(() => setText(saved), [saved]);

  async function save(value: string) {
    setBusy(true);
    setError(null);
    setOk(false);
    try {
      upsert(await api.setInstanceJvmArgs(instance.id, value));
      setOk(true);
    } catch (e) {
      setError(toCmdError(e));
    } finally {
      setBusy(false);
    }
  }

  const dirty = text.trim() !== saved.trim();
  return (
    <Card title="JVM arguments">
      <label htmlFor={id} className="sr-only">
        JVM arguments
      </label>
      <textarea
        id={id}
        aria-describedby={hint}
        aria-invalid={error ? true : undefined}
        value={text}
        disabled={disabled}
        spellCheck={false}
        rows={3}
        placeholder="-XX:+UseG1GC -XX:MaxGCPauseMillis=50"
        onChange={(e) => {
          setText(e.target.value);
          setOk(false);
        }}
        className="w-full resize-y rounded-md border border-line bg-surface-2 px-3 py-2 font-mono text-xs text-fg placeholder:text-fg-faint hover:border-line-strong focus:border-accent focus:outline-none disabled:opacity-50"
      />
      <p id={hint} className="mt-2 text-xs leading-relaxed text-fg-faint">
        Advanced. Allowed: <code className="font-mono">-XX:…</code>, <code className="font-mono">-D…</code>, <code className="font-mono">-Xss</code>,{" "}
        <code className="font-mono">-Xmn</code>, <code className="font-mono">-Xlog</code>, <code className="font-mono">--add-opens</code>/
        <code className="font-mono">--add-exports</code>, <code className="font-mono">--enable-native-access</code>. Memory is set above; agents,
        classpath changes and command-running flags are refused.
      </p>
      {error && (
        <div className="mt-3">
          <InlineError error={error} />
        </div>
      )}
      <div className="mt-3 flex items-center justify-end gap-2">
        {ok && !dirty && (
          <span role="status" className="mr-auto text-xs text-success">
            Saved. Applies the next time you play.
          </span>
        )}
        {instance.jvmArgs.length > 0 && (
          <Button size="sm" variant="ghost" disabled={disabled || busy} onClick={() => void save("")}>
            Clear
          </Button>
        )}
        <Button size="sm" variant="primary" disabled={disabled || busy || !dirty} onClick={() => void save(text)}>
          Save
        </Button>
      </div>
    </Card>
  );
}
