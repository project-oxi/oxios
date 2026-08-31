import i18n from 'i18next'
import LanguageDetector from 'i18next-browser-languagedetector'
import { initReactI18next } from 'react-i18next'
import en from './locales/en.json'
import ko from './locales/ko.json'

/**
 * i18n bootstrap.
 *
 * All translation keys live in a single flat JSON per language, registered
 * under the `common` namespace. The `keySeparator: '.'` (i18next default)
 * lets call-sites write `t('sidebar.operate')` which resolves to
 * `common.sidebar.operate` — every top-level JSON key is a dot-path root.
 *
 * The `i18next.config.schema.json` lists aspirational namespaces for
 * tooling (i18n-ally, extractors); they are NOT used at runtime.
 */
const resources = {
  en: { common: en },
  ko: { common: ko },
}

export const initI18n = () => {
  i18n
    .use(LanguageDetector)
    .use(initReactI18next)
    .init({
      resources,
      fallbackLng: 'en',
      supportedLngs: ['en', 'ko'],
      defaultNS: 'common',
      ns: ['common'],
      interpolation: {
        escapeValue: false,
      },
      detection: {
        order: ['localStorage', 'navigator'],
        caches: ['localStorage'],
        lookupLocalStorage: 'i18nextLng',
      },
    })

  return i18n
}

export default i18n
