import { create } from 'zustand';

export type DialogId = 'about' | 'toolchain' | 'toolchainHelp' | 'fuses' | 'supply' | 'speed' | 'update' | 'gotoAddress' | 'customDevice' | 'whatsNew';

interface DialogStore {
  open: DialogId | null;
}

export const useDialogs = create<DialogStore>(() => ({ open: null }));

export function openDialog(id: DialogId): void {
  useDialogs.setState({ open: id });
}

export function closeDialog(): void {
  useDialogs.setState({ open: null });
}
