export const API_URL = import.meta.env.VITE_API_URL ?? 'http://localhost:6001';
export const TURNSTILE_SITE_KEY = import.meta.env.VITE_TURNSTILE_SITE_KEY ?? '';

type ApiResponse<T> = {
	error: string | null;
	data: T | null;
};

/** Thrown by `api()` on a non-2xx response. Carries `captchaRequired` when the backend
 *  flags that the next attempt on this endpoint needs a Turnstile token (adaptive login
 *  bot-check) — plain `Error` callers are unaffected since this still `instanceof Error`.
 *  `status` lets callers tell "you may not see this" (403) from a real failure without
 *  matching on the message text. */
export class ApiError extends Error {
	captchaRequired?: boolean;
	status: number;
	/** Machine-readable reason beside the message, when the API gives one. */
	code?: string;
	constructor(message: string, status: number, captchaRequired?: boolean, code?: string) {
		super(message);
		this.name = 'ApiError';
		this.status = status;
		this.captchaRequired = captchaRequired;
		this.code = code;
	}
}

/** The API refuses an organization route when the organization requires
 *  two-factor authentication and the account has none. */
export const TWO_FACTOR_REQUIRED = 'two_factor_required';

/** Sends the user to set up 2FA, remembering where they were so setup can
 *  bring them back. Not from the account page itself, which has to stay
 *  reachable to do the setup. */
function redirectToTwoFactorSetup() {
	if (typeof window === 'undefined' || window.location.pathname.startsWith('/account')) return;
	const next = `${window.location.pathname}${window.location.search}`;
	window.location.assign(
		`/account?tab=security&two_factor=required&next=${encodeURIComponent(next)}`,
	);
}

async function handleResponse<T>(res: Response): Promise<ApiResponse<T>> {
	// Session expired / not authenticated — bounce to the login route.
	// The reset page gets 401 for an expired link and shows its own message.
	if (
		res.status === 401 &&
		typeof window !== 'undefined' &&
		window.location.pathname !== '/reset-password'
	) {
		if (window.location.pathname !== '/login') {
			const next = `${window.location.pathname}${window.location.search}`;
			window.location.assign(`/login?next=${encodeURIComponent(next)}`);
		}
		throw new Error('Session expired. Please log in again.');
	}

	// Empty-body responses (204, or bare error statuses) have no JSON to parse.
	const raw = await res.text();
	if (!raw) {
		if (!res.ok) {
			throw new Error(`${res.status} ${res.statusText}`);
		}
		return { error: null, data: null as T };
	}

	const body = JSON.parse(raw);
	if (!res.ok) {
		if (res.status === 403 && body.code === TWO_FACTOR_REQUIRED) redirectToTwoFactorSetup();
		throw new ApiError(
			body.error || `${res.status} ${res.statusText}`,
			res.status,
			body.captcha_required,
			body.code,
		);
	}
	return body;
}

export async function api<T>(path: string, opts?: RequestInit): Promise<ApiResponse<T>> {
	const res = await fetch(`${API_URL}${path}`, {
		...opts,
		credentials: 'include',
		headers: {
			'Content-Type': 'application/json',
			...opts?.headers,
		},
	});
	return handleResponse<T>(res);
}

/** Like `api()`, but for a `FormData` body (file upload) — skips the JSON
 *  `Content-Type` header so the browser sets its own multipart boundary. */
export async function apiForm<T>(
	path: string,
	body: FormData,
	method = 'POST',
): Promise<ApiResponse<T>> {
	const res = await fetch(`${API_URL}${path}`, { method, credentials: 'include', body });
	return handleResponse<T>(res);
}
