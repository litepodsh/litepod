import { UAParser } from 'ua-parser-js';

export type DeviceInfo = {
	osKey: 'macos' | 'ios' | 'windows' | 'linux' | 'android' | 'other';
	osLabel: string;
	browserKey: 'chrome' | 'firefox' | 'safari' | 'edge' | 'opera' | 'other';
	browserLabel: string;
	deviceType: 'mobile' | 'tablet' | 'desktop' | 'other';
};

/**
 * Parse a raw User-Agent string into normalized OS + browser info.
 * Returns null-friendly defaults when UA is missing or unparseable.
 */
export function parseDevice(userAgent: string | null | undefined): DeviceInfo {
	const fallback: DeviceInfo = {
		osKey: 'other',
		osLabel: 'Unknown device',
		browserKey: 'other',
		browserLabel: 'Unknown browser',
		deviceType: 'other',
	};

	if (!userAgent) return fallback;

	const parsed = new UAParser(userAgent).getResult();
	const osName = parsed.os.name?.toLowerCase() ?? '';
	const browserName = parsed.browser.name?.toLowerCase() ?? '';
	const rawDeviceType = parsed.device.type;
	let deviceType: DeviceInfo['deviceType'] = 'other';
	if (rawDeviceType === 'mobile') deviceType = 'mobile';
	else if (rawDeviceType === 'tablet') deviceType = 'tablet';
	else if (rawDeviceType === undefined) deviceType = 'desktop';

	let osKey: DeviceInfo['osKey'] = 'other';
	let osLabel = parsed.os.name ?? 'Unknown OS';
	if (osName.includes('mac os')) {
		osKey = 'macos';
		osLabel = 'macOS';
	} else if (osName.includes('ios')) {
		osKey = 'ios';
		osLabel = 'iOS';
	} else if (osName.includes('windows')) {
		osKey = 'windows';
		osLabel = 'Windows';
	} else if (osName.includes('android')) {
		osKey = 'android';
		osLabel = 'Android';
	} else if (osName.includes('linux') || osName.includes('ubuntu')) {
		osKey = 'linux';
		osLabel = 'Linux';
	}

	let browserKey: DeviceInfo['browserKey'] = 'other';
	let browserLabel = parsed.browser.name ?? 'Unknown browser';
	if (
		browserName.includes('chrome') &&
		!browserName.includes('chromium') &&
		!browserName.includes('edge')
	) {
		browserKey = 'chrome';
		browserLabel = 'Chrome';
	} else if (browserName.includes('firefox')) {
		browserKey = 'firefox';
		browserLabel = 'Firefox';
	} else if (browserName.includes('safari') && !browserName.includes('chrome')) {
		browserKey = 'safari';
		browserLabel = 'Safari';
	} else if (browserName.includes('edge') || browserName.includes('edg/')) {
		browserKey = 'edge';
		browserLabel = 'Edge';
	} else if (browserName.includes('opera') || browserName.includes('opr/')) {
		browserKey = 'opera';
		browserLabel = 'Opera';
	}

	return { osKey, osLabel, browserKey, browserLabel, deviceType };
}
