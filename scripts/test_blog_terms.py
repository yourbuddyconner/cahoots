import unittest
from blog_terms import TermDefinitions


class FirstMentionTests(unittest.TestCase):
    def explain(self, body):
        parser = TermDefinitions({
            'grant': {'forms': ['grant', 'grants'], 'definition': 'A signed permission.'},
            'public key': {'forms': ['public key', 'public keys'], 'definition': 'A shareable key.'},
        })
        parser.feed(body)
        parser.close()
        return ''.join(parser.output), parser.count

    def test_first_use_only_in_the_main_reading_path(self):
        body, count = self.explain('<p>Grants use a public key.</p><p>A grant has limits. Public keys are shared.</p>')
        self.assertEqual(count, 2)
        self.assertIn('<strong>Grants</strong>', body)
        self.assertIn('<p>A grant has limits. Public keys are shared.</p>', body)

    def test_code_links_headings_and_controls_keep_their_markup(self):
        original = '<h2>Grants</h2><a href="/grant">grant</a><code>grant</code><summary>grant</summary><svg><text>grant</text></svg>'
        body, count = self.explain(original + '<p>grant</p>')
        self.assertTrue(body.startswith(original))
        self.assertEqual(count, 1)

    def test_an_optional_aside_does_not_consume_the_main_introduction(self):
        body, count = self.explain('<details><summary>Details</summary><p>A grant.</p></details><p>A grant.</p>')
        self.assertEqual(count, 2)
        self.assertIn('aria-describedby="term-1"', body)
        self.assertIn('aria-describedby="term-2"', body)

    def test_existing_entities_and_attributes_are_preserved(self):
        original = '<p id="example" data-note="public key">A&amp;B&#39;s grant is limited.</p>'
        body, count = self.explain(original)
        self.assertIn('id="example" data-note="public key"', body)
        self.assertIn('A&amp;B&#39;s', body)
        self.assertEqual(count, 1)

    def test_glossary_text_cannot_inject_html(self):
        parser = TermDefinitions({'grant': {'forms': ['grant'], 'definition': '<script>bad()</script>'}})
        parser.feed('<p>grant</p>')
        self.assertIn('&lt;script&gt;bad()&lt;/script&gt;', ''.join(parser.output))


if __name__ == '__main__':
    unittest.main()
