/** Hooks the active CodeMirror view registers so menus/toolbars can drive the editor. */
export interface EditorApi {
  undo(): void;
  redo(): void;
  cut(): void;
  copy(): void;
  paste(): void;
  selectAll(): void;
  find(): void;
  replace(): void;
  gotoLine(): void;
  /** 1-based line of the main cursor. */
  cursorLine(): number;
  focus(): void;
  /** Inserts text at the cursor (replacing the selection). */
  insertText(text: string): void;
  /** Symbol View: shows symbol `index` of the outline (-1 = top of file). */
  focusSymbol(index: number): void;
  /** Symbol View: adds a symbol after the shown one. */
  addSymbol(name: string): void;
}

let current: EditorApi | null = null;

export function setEditorApi(api: EditorApi | null): void {
  current = api;
}

export function editorApi(): EditorApi | null {
  return current;
}
