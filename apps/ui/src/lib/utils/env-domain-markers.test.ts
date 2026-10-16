import { describe, expect, it } from 'vitest';
import { domainMarkerPresets, isDomainMarker } from './env-domain-markers';

describe('domainMarkerPresets', () => {
	it('offers the primary forms plus one per catalog port', () => {
		expect(domainMarkerPresets([8080, 3002])).toEqual([
			'{{url}}',
			'{{domain}}',
			'{{url:8080}}',
			'{{domain:8080}}',
			'{{url:3002}}',
			'{{domain:3002}}',
		]);
	});
});

describe('isDomainMarker', () => {
	it('matches only url/domain markers', () => {
		expect(isDomainMarker('{{url}}')).toBe(true);
		expect(isDomainMarker('{{ domain:8080 }}')).toBe(true);
		expect(isDomainMarker('{{domain:web:3000}}')).toBe(true);
		expect(isDomainMarker('{{password_32}}')).toBe(false);
		expect(isDomainMarker('{{urlx}}')).toBe(false);
	});
});
