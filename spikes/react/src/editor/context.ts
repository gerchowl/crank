import { createContext, use, type Dispatch } from "react";
import type { CompiledWorkout } from "@neo/shared";
import type { EditorAction, EditorState } from "./state";

export interface EditorCtx { state: EditorState; dispatch: Dispatch<EditorAction>; compiled: CompiledWorkout }
export const EditorContext = createContext<EditorCtx | null>(null);

export function useEditor(): EditorCtx {
  const c = use(EditorContext);
  if (!c) throw new Error("useEditor outside <EditorContext>");
  return c;
}
