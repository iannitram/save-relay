// What the views ask the Bridge to do. Plain async functions, so tests drive them with a fake
// Bridge. Each returns the envelope's data, or throws `PanelError` with the error's message.

import type { Answer, AnswerData, Envelope, RestoreData, SignInPrompt, SyncReport } from "./api.ts";
import type { ApiArgs, ApiCommand, Bridge } from "./bridge.ts";

export class PanelError extends Error {
  code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

export async function call<T>(bridge: Bridge, command: ApiCommand, args: ApiArgs): Promise<T> {
  let envelope: Envelope<T>;
  try {
    envelope = await bridge.call<T>(command, args);
  } catch (e) {
    throw new PanelError("io", e instanceof Error ? e.message : String(e));
  }
  if (!envelope.ok) throw new PanelError(envelope.error.code, envelope.error.message);
  return envelope.data;
}

/** The command line's spelling of an answer: `keep_server` is `keep-server`. */
export function cliAnswer(answer: Answer): string {
  return answer.replace("_", "-");
}

/** A choice made on a pending choice (`answer`) or on a `pull` that came back held (`pull --anyway`). */
export interface ChoiceTarget {
  game: string;
  /** True when `status` reported it as pending, so `answer` has something to write to. */
  pending: boolean;
}

export function choose(
  bridge: Bridge,
  target: ChoiceTarget,
  answer: Answer,
): Promise<AnswerData | SyncReport> {
  if (!target.pending && answer === "play_anyway") {
    return call<SyncReport>(bridge, "pull", { game: target.game, anyway: true });
  }
  return call<AnswerData>(bridge, "answer", { game: target.game, answer: cliAnswer(answer) });
}

export function restore(bridge: Bridge, game: string, entry: string): Promise<RestoreData> {
  return call<RestoreData>(bridge, "restore", { game, entry });
}

/** Google Drive or Dropbox sign-in, as the prompt to show (`setup --no-browser`). */
export function signIn(bridge: Bridge, service: "gdrive" | "dropbox"): Promise<SignInPrompt> {
  return call<SignInPrompt>(bridge, "setup", { service, "no-browser": true });
}
