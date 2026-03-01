import { writable } from "svelte/store";
import type { Toast, ToastType } from "../types";

const { subscribe, update } = writable<Toast[]>([]);

function addToast(type: ToastType, title: string, message?: string, duration = 4000) {
  const id = crypto.randomUUID();
  const t: Toast = { id, type, title, message, duration };
  update(toasts => [...toasts, t]);
  if (duration > 0) setTimeout(() => removeToast(id), duration);
  return id;
}

function removeToast(id: string) {
  update(toasts => toasts.filter(t => t.id !== id));
}

export const toasts = { subscribe };
export const toast = {
  info: (title: string, msg?: string) => addToast("info", title, msg),
  success: (title: string, msg?: string) => addToast("success", title, msg),
  warning: (title: string, msg?: string) => addToast("warning", title, msg),
  error: (title: string, msg?: string) => addToast("error", title, msg, 8000),
  switch: (title: string, msg?: string) => addToast("switch", title, msg, 5000),
  remove: removeToast,
};
