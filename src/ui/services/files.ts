/** Document lifecycle: new / open / save / close, examples and session restore. */
import { confirmDialog, pickFile, pickSavePath, readTextFile, writeTextFile } from '../backend/api';
import { getDocText, initialTexts } from '../editor/docText';
import { useSettings } from '../state/settings';
import { activeDoc, addDoc, appendOutput, closeDoc, languageFor, untitledName, updateDoc, useWorkspace, type Doc } from '../state/workspace';
import { baseName } from './debugInfo';
import { EXAMPLES } from './examples';

const SOURCE_FILTERS = [
  { name: 'Source files', extensions: ['asm', 's', 'S', 'inc', 'c', 'h', 'cpp'] },
  { name: 'Assembly (avrasm2)', extensions: ['asm', 'inc'] },
  { name: 'C / C++', extensions: ['c', 'h', 'cpp'] },
  { name: 'All files', extensions: ['*'] },
];

export function newFile(kind: 'asm' | 'c'): void {
  const ex = EXAMPLES.find((e) => e.template === kind);
  addDoc(untitledName(kind === 'asm' ? '.asm' : '.c'), null, ex?.text ?? '', kind);
}

export async function openFileDialog(): Promise<void> {
  const path = await pickFile('Open Source File', SOURCE_FILTERS);
  if (path) await openPath(path);
}

export async function openPath(path: string): Promise<boolean> {
  try {
    const text = await readTextFile(path);
    addDoc(baseName(path), path, text);
    useSettings.getState().addRecent(path);
    persistOpenFiles();
    return true;
  } catch (e) {
    appendOutput('error', `Could not open ${path}: ${e instanceof Error ? e.message : String(e)}`);
    return false;
  }
}

export function openExample(name: string): void {
  const ex = EXAMPLES.find((e) => e.name === name);
  if (ex) addDoc(ex.name, null, ex.text);
}

export async function saveDoc(doc: Doc | undefined = activeDoc(), forceDialog = false): Promise<boolean> {
  if (!doc) return false;
  let path = doc.path;
  if (!path || forceDialog) {
    path = await pickSavePath('Save As', doc.path ?? doc.name, SOURCE_FILTERS);
    if (!path) return false;
  }
  try {
    await writeTextFile(path, getDocText(doc.id));
  } catch (e) {
    appendOutput('error', `Could not save ${path}: ${e instanceof Error ? e.message : String(e)}`);
    return false;
  }
  updateDoc(doc.id, { path, name: baseName(path), dirty: false, language: languageFor(baseName(path)) });
  useSettings.getState().addRecent(path);
  persistOpenFiles();
  return true;
}

export async function saveAll(): Promise<void> {
  for (const d of useWorkspace.getState().docs) if (d.dirty) await saveDoc(d);
}

export async function closeDocument(id: string | null | undefined = useWorkspace.getState().activeDocId): Promise<boolean> {
  const doc = useWorkspace.getState().docs.find((d) => d.id === id);
  if (!doc) return true;
  if (doc.dirty) {
    const save = await confirmDialog(`Save changes to ${doc.name} before closing?\n\nChoose "No" to discard them.`, 'Unsaved changes');
    if (save && !(await saveDoc(doc))) return false;
  }
  closeDoc(doc.id);
  persistOpenFiles();
  return true;
}

/** Returns true when it is OK to quit (asks about unsaved documents). */
export async function confirmQuit(): Promise<boolean> {
  const dirty = useWorkspace.getState().docs.filter((d) => d.dirty);
  if (dirty.length === 0) return true;
  const save = await confirmDialog(`Save changes to ${dirty.map((d) => d.name).join(', ')} before exiting?\n\nChoose "No" to discard them.`, 'Unsaved changes');
  if (save) {
    for (const d of dirty) if (!(await saveDoc(d))) return false;
  }
  return true;
}

export function persistOpenFiles(): void {
  const paths = useWorkspace.getState().docs.map((d) => d.path).filter((p): p is string => !!p);
  useSettings.getState().set({ openFiles: paths });
}

export async function restoreSession(): Promise<void> {
  for (const p of useSettings.getState().openFiles) await openPath(p);
}

/** Marks a document dirty when its text differs from what was loaded/saved. */
export function markDirty(id: string): void {
  const d = useWorkspace.getState().docs.find((x) => x.id === id);
  if (d && !d.dirty) updateDoc(id, { dirty: true });
  initialTexts.delete(id);
}
