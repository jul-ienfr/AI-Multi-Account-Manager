import { writable, get } from 'svelte/store';
import fr from './locales/fr.json';
import en from './locales/en.json';

export type Locale = 'fr' | 'en';

const locales: Record<Locale, Record<string, string>> = { fr, en };

// ---------------------------------------------------------------------------
// Reactive store — uses Svelte writable store (works in plain .ts files).
// ---------------------------------------------------------------------------

const initial: Locale =
  (typeof localStorage !== 'undefined'
    ? (localStorage.getItem('locale') as Locale | null)
    : null) ?? 'fr';

export const i18nStore = writable<Locale>(initial);

/** Change the active locale and persist the choice. */
export function setLocale(locale: Locale): void {
  i18nStore.set(locale);
  localStorage.setItem('locale', locale);
}

/** Return the currently active locale. */
export function getLocale(): Locale {
  return get(i18nStore);
}

/**
 * Translate a key with optional parameter interpolation.
 *
 * @example
 * t('toast.switch_success', { account: 'john@example.com' })
 * // → "Switch vers john@example.com"  (fr)
 * // → "Switched to john@example.com" (en)
 */
export function t(key: string, params?: Record<string, string | number>): string {
  const dict = locales[get(i18nStore)] ?? locales['fr'];
  let value: string = dict[key] ?? key;
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      value = value.replaceAll(`{${k}}`, String(v));
    }
  }
  return value;
}
