/**
 * Splits a settings-style page's sections into the groups the tab capsule shows
 * and the sections of the open group. Groups left empty by permissions or
 * edition disappear; `current` must be one of `sections`.
 */
export function groupSections<
	Group extends { value: string },
	Section extends { value: string; group: string },
>(groups: Group[], sections: Section[], current: string) {
	const group = sections.find((section) => section.value === current)!.group;
	return {
		groups: groups.filter((item) => sections.some((section) => section.group === item.value)),
		group,
		items: sections.filter((section) => section.group === group),
	};
}
