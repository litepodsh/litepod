export function maskConnectionStringPassword(value: string): string {
	const authorityStart = value.indexOf('://');
	if (authorityStart < 0) return maskKeywordConnectionStringPassword(value);

	const userInfoStart = authorityStart + 3;
	const authorityEndCandidates = ['/', '?', '#']
		.map((separator) => value.indexOf(separator, userInfoStart))
		.filter((index) => index >= 0);
	const authorityEnd =
		authorityEndCandidates.length > 0 ? Math.min(...authorityEndCandidates) : value.length;
	const userInfoEnd = value.lastIndexOf('@', authorityEnd - 1);
	if (userInfoEnd < userInfoStart) return value;

	const passwordStart = value.indexOf(':', userInfoStart);
	if (passwordStart < 0 || passwordStart > userInfoEnd) return value;

	return `${value.slice(0, passwordStart + 1)}••••${value.slice(userInfoEnd)}`;
}

/** ADO/ODBC keyword strings (`Server=…;User Id=…;Password=…;`) carry no URI authority to parse. */
function maskKeywordConnectionStringPassword(value: string): string {
	return value.replace(/(^|;)(\s*(?:password|pwd)\s*=)[^;]*/gi, '$1$2••••');
}
