// Regression: rehype-highlight wraps every token in `<span class="hljs-*">`
// before react-markdown invokes the `code` component. Flattening the React
// children to a string dropped every highlighted token, so `fn main() {
// println!("hi"); }` rendered as ` () { (); }` and Copy copied that corruption.
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { MarkdownMessage } from '@/components/chat/markdown-message'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { count?: number }) =>
      opts && typeof opts.count === 'number' ? `${key}:${opts.count}` : key,
    i18n: { language: 'en' },
  }),
}))

describe('MarkdownMessage code blocks', () => {
  it('keeps every token of a language-tagged block', () => {
    const md = ['```rust', 'fn main() { println!("hi"); }', '```'].join('\n')
    const { container } = render(<MarkdownMessage>{md}</MarkdownMessage>)
    const code = container.querySelector('code.language-rust')
    expect(code?.textContent).toBe('fn main() { println!("hi"); }\n')
  })

  it('applies syntax highlighting spans', () => {
    const md = ['```rust', 'fn main() {}', '```'].join('\n')
    const { container } = render(<MarkdownMessage>{md}</MarkdownMessage>)
    expect(container.querySelectorAll('code.language-rust .hljs-keyword').length).toBeGreaterThan(0)
  })

  it('renders a plain fenced block without a language', () => {
    const md = ['```', 'plain text body', '```'].join('\n')
    render(<MarkdownMessage>{md}</MarkdownMessage>)
    expect(screen.getByText(/plain text body/)).toBeTruthy()
  })

  it('renders inline code inline, not as a code-block card', () => {
    const { container } = render(<MarkdownMessage>{'use `cargo test` now'}</MarkdownMessage>)
    expect(container.querySelector('pre')).toBeNull()
    expect(container.querySelector('p > code')?.textContent).toBe('cargo test')
  })

  it('emits highlight classes that the theme layer can style', () => {
    const { container } = render(<MarkdownMessage>{'```rust\nfn main() {}\n```'}</MarkdownMessage>)
    expect(container.querySelector('.hljs-keyword')).toBeTruthy()
  })

  it('collapses a very long block behind an expand control', () => {
    const body = Array.from({ length: 60 }, (_, i) => `line ${i}`).join('\n')
    const { container } = render(<MarkdownMessage>{`\`\`\`text\n${body}\n\`\`\``}</MarkdownMessage>)
    expect(container.querySelector('[data-collapsed="true"]')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /expand/i }))
    expect(container.querySelector('[data-collapsed="true"]')).toBeNull()
  })
})
