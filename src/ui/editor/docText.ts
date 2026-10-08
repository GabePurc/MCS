/**
 * Document text lives outside React state: CodeMirror EditorStates (with undo history) are kept
 * per document here; documents never shown in the editor keep their plain initial text.
 */
import type { EditorState } from '@codemirror/state';

export const initialTexts = new Map<string, string>();
export const editorStates = new Map<string, EditorState>();

export function getDocText(id: string): string {
  const st = editorStates.get(id);
  if (st) return st.doc.toString();
  return initialTexts.get(id) ?? '';
}

export function forgetDoc(id: string): void {
  initialTexts.delete(id);
  editorStates.delete(id);
}
