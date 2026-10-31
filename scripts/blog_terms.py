"""Explain the first use of journal vocabulary without changing links, code, or headings."""
import html
import json
import re
from html.parser import HTMLParser
from pathlib import Path


class TermDefinitions(HTMLParser):
    protected = {"a", "abbr", "dfn", "code", "pre", "script", "style", "svg", "summary", "h1", "h2", "h3", "h4"}

    def __init__(self, terms):
        super().__init__(convert_charrefs=False)
        self.terms = terms
        self.forms = {form.casefold(): name for name, term in terms.items() for form in term["forms"]}
        choices = sorted(self.forms, key=len, reverse=True)
        self.pattern = re.compile(r"(?<!\w)(?:" + "|".join(re.escape(form) for form in choices) + r")(?!\w)", re.I)
        self.seen = [set()]
        self.skip = []
        self.output = []
        self.count = 0

    def handle_starttag(self, tag, attrs):
        self.output.append(self.get_starttag_text())
        if tag in self.protected:
            self.skip.append(tag)
        if tag == "details":
            # An optional detour can introduce a term without consuming its first main-text use.
            self.seen.append(set(self.seen[-1]))

    def handle_endtag(self, tag):
        self.output.append(f"</{tag}>")
        if tag in self.skip:
            self.skip.remove(tag)
        if tag == "details" and len(self.seen) > 1:
            self.seen.pop()

    def handle_startendtag(self, tag, attrs):
        self.output.append(self.get_starttag_text())

    def handle_data(self, data):
        if self.skip or not self.forms:
            self.output.append(data)
            return

        def explain(match):
            name = self.forms[match.group().casefold()]
            if name in self.seen[-1]:
                return match.group()
            self.seen[-1].add(name)
            self.count += 1
            identity = f"term-{self.count}"
            definition = html.escape(self.terms[name]["definition"])
            word = html.escape(match.group())
            return (f'<span class="defined-term" tabindex="0" aria-describedby="{identity}">'
                    f'<dfn><strong>{word}</strong></dfn>'
                    f'<span class="term-definition" id="{identity}" role="tooltip">{definition}</span></span>')
        self.output.append(self.pattern.sub(explain, data))

    def handle_entityref(self, name):
        self.output.append(f"&{name};")

    def handle_charref(self, name):
        self.output.append(f"&#{name};")

    def handle_comment(self, data):
        self.output.append(f"<!--{data}-->")


def explain_terms(body, glossary):
    terms = json.loads(Path(glossary).read_text(encoding="utf-8"))
    parser = TermDefinitions(terms)
    parser.feed(body)
    parser.close()
    return "".join(parser.output), parser.count
