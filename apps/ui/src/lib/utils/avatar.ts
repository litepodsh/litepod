export function avatarUrl(username: string): string {
	const seed = username.replace(/[^a-zA-Z0-9]/g, '').slice(0, 20) || 'user';
	const params = new URLSearchParams({
		theme: 'micah',
		seed,
		size: '64',
	});
	return `https://avatune.sebasgc.xyz/?${params}`;
}

export function initials(name: string): string {
	return name
		.split(/\s+/)
		.map((n) => n[0]?.toUpperCase() ?? '')
		.slice(0, 2)
		.join('');
}
