+++
title = "Math Rendering"
weight = 90
+++

Zola can render Markdown math formulas with [Typst](https://typst.app/) and output semantic [MathML](https://developer.mozilla.org/en-US/docs/Web/MathML).

Math rendering is disabled by default. Enable it in your `zola.toml`:

```toml
[markdown.math]
enabled = true
engine = "typst"
```

## Syntax

Zola's built-in math renderer uses **Typst math syntax**, not LaTeX syntax.

Inline formulas are written with single dollar delimiters:

```md
The Pythagorean theorem is $a^2 + b^2 = c^2$.
```

Display formulas are written with double dollar delimiters:

```md
$$
integral_0^1 x^2 dif x = 1 / 3
$$
```

## Typst preamble

You can prepend Typst definitions to every formula with `preamble`:

```toml
[markdown.math]
enabled = true
engine = "typst"

[markdown.math.typst]
preamble = "#let sq(x) = $ #x^2 $"
```

Then Markdown formulas can use those definitions:

```md
The square is $sq(a)$.
```

For longer preambles, use a file path relative to the directory containing `config.toml`:

```toml
[markdown.math.typst]
preamble_file = "typst-math.typ"
```

If both `preamble` and `preamble_file` are set, Zola prepends the inline `preamble` first, followed by the file contents.

## HTML output

The generated HTML contains MathML, wrapped in Zola-specific classes:

```html
<span class="zola-math zola-math-inline">
  <math>...</math>
</span>
```

and for display math:

```html
<div class="zola-math zola-math-display">
  <math display="block">...</math>
</div>
```

## Styling

Browsers render MathML natively. Themes can target the wrapper classes to adjust spacing or overflow behavior:

```css
.zola-math-display {
  margin: 1rem 0;
  overflow-x: auto;
}

.zola-math-inline math {
  vertical-align: middle;
}
```

## LaTeX compatibility

The built-in renderer expects Typst syntax. LaTeX input such as `\frac{1}{2}` is not supported by this setting.

Support for LaTeX through Typst packages such as `mitex` may be added in a future release.
