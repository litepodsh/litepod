import { expect, it } from 'vitest';
import { renderMarkdown } from './markdown';

it('renders the structure the model actually writes', () => {
	const html = renderMarkdown('Set `PORT` to 8080.\n\n- one\n- two');
	expect(html).toContain('<code>PORT</code>');
	expect(html).toContain('<li>one</li>');
});

it('drops a script a log line smuggled into a reply', () => {
	const html = renderMarkdown('before <script>alert(1)<\/script> after');
	expect(html).not.toContain('<script');
	expect(html).toContain('before');
});

it('drops images, so a reply cannot make the panel fetch a URL', () => {
	const html = renderMarkdown('![pixel](https://tracker.example/p.gif)');
	expect(html).not.toContain('<img');
});

it('drops inline event handlers', () => {
	const html = renderMarkdown('<a href="#" onclick="alert(1)">click</a>');
	expect(html).not.toContain('onclick');
});
