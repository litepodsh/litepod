type ExplicitPrivateNetworkResult =
	| { content: string; kind: 'updated' }
	| { kind: 'already-configured' | 'unsupported' };

/**
 * Makes Litepod's generated private network part of a simple Compose file.
 * Complex, intentionally custom network topologies stay untouched for the
 * user to edit directly.
 */
export function makePrivateNetworkExplicit(
	content: string,
	applicationId: string,
): ExplicitPrivateNetworkResult {
	const lines = content.split('\n');
	const servicesStart = lines.findIndex((line) => /^services:\s*(?:#.*)?$/.test(line));
	if (servicesStart === -1) return { kind: 'unsupported' };

	const serviceLines: number[] = [];
	let servicesEnd = lines.length;
	for (let index = servicesStart + 1; index < lines.length; index += 1) {
		const line = lines[index];
		if (/^\S/.test(line) && !line.startsWith('#')) {
			servicesEnd = index;
			break;
		}
		if (/^  [^\s#][^:]*:\s*(?:#.*)?$/.test(line)) serviceLines.push(index);
	}
	if (serviceLines.length === 0) return { kind: 'unsupported' };

	for (const [position, start] of serviceLines.entries()) {
		const end = serviceLines[position + 1] ?? servicesEnd;
		if (lines.slice(start + 1, end).some((line) => /^    (networks|network_mode):/.test(line))) {
			return { kind: 'already-configured' };
		}
	}

	const rootNetworks = lines.findIndex((line) => /^networks:\s*(?:#.*)?$/.test(line));
	if (rootNetworks !== -1) return { kind: 'already-configured' };

	for (const [position, start] of [...serviceLines.entries()].reverse()) {
		const nextStart = serviceLines[position + 1] ?? servicesEnd;
		let blankCount = 0;
		while (nextStart - blankCount > start + 1 && /^\s*$/.test(lines[nextStart - blankCount - 1])) {
			blankCount += 1;
		}
		lines.splice(nextStart - blankCount, blankCount, '    networks:', '      private: {}');
	}

	while (lines.length > 0 && lines.at(-1)?.trim() === '') lines.pop();
	lines.push('', 'networks:', '  private:', `    name: ${applicationId}_private`);
	return { content: lines.join('\n'), kind: 'updated' };
}
