import { catalogs, type Catalogs, type Locale } from './catalogs';

const STORAGE_KEY = 'litepod:locale';
const EUROPE_REGIONS = new Set([
	'AL',
	'AD',
	'AT',
	'BA',
	'BE',
	'BG',
	'BY',
	'CH',
	'CY',
	'CZ',
	'DE',
	'DK',
	'EE',
	'ES',
	'FI',
	'FO',
	'FR',
	'GB',
	'GG',
	'GI',
	'GR',
	'HR',
	'HU',
	'IE',
	'IM',
	'IS',
	'IT',
	'JE',
	'LI',
	'LT',
	'LU',
	'LV',
	'MC',
	'MD',
	'ME',
	'MK',
	'MT',
	'NL',
	'NO',
	'PL',
	'PT',
	'RO',
	'RS',
	'SE',
	'SI',
	'SJ',
	'SK',
	'SM',
	'UA',
	'VA',
]);

export const i18n = $state({ locale: 'en' as Locale, europeanFlags: false });

export function prefersEuropeanFlags(browserLanguages: readonly string[] = []) {
	return browserLanguages.some((language) => {
		const region = language.split('-')[1]?.toUpperCase();
		return region !== undefined && EUROPE_REGIONS.has(region);
	});
}

export function detectLocale(
	storageValue: string | null | undefined,
	browserLanguages: readonly string[] = [],
): Locale {
	if (storageValue === 'en' || storageValue === 'es') return storageValue;
	return browserLanguages.some((language) => language.toLowerCase().split('-')[0] === 'es')
		? 'es'
		: 'en';
}

export function restoreLocale() {
	if (typeof window === 'undefined') return;
	try {
		i18n.locale = detectLocale(localStorage.getItem(STORAGE_KEY), navigator.languages);
		i18n.europeanFlags = prefersEuropeanFlags(navigator.languages);
	} catch {
		i18n.locale = 'en';
		i18n.europeanFlags = false;
	}
}

export function setLocale(locale: Locale) {
	i18n.locale = locale;
	try {
		localStorage.setItem(STORAGE_KEY, locale);
	} catch {
		// Storage is optional; the in-memory selection still applies.
	}
}

export function createTranslator(source: Catalogs, selected: () => Locale) {
	return (key: string, variables: Record<string, unknown> = {}) => {
		const active = selected();
		const fallback: Locale = active === 'en' ? 'es' : 'en';
		const message = source[active][key] ?? source[fallback][key] ?? key;

		return message.replace(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g, (placeholder, name: string) =>
			Object.hasOwn(variables, name) ? String(variables[name]) : placeholder,
		);
	};
}

export const t = createTranslator(catalogs, () => i18n.locale);
