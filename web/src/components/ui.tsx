import { Check, Copy, Loader2 } from 'lucide-react'
import { useState, type ButtonHTMLAttributes, type ReactNode } from 'react'
import { cx } from '../lib/style'

export function Card({
  title,
  icon,
  action,
  children,
  className,
}: {
  title?: ReactNode
  icon?: ReactNode
  action?: ReactNode
  children: ReactNode
  className?: string
}) {
  return (
    <section
      className={cx(
        'rounded-xl border border-zinc-200 bg-white p-4 shadow-sm dark:border-zinc-800 dark:bg-zinc-900',
        className,
      )}
    >
      {(title || action) && (
        <header className="mb-3 flex items-center justify-between gap-2">
          <h2 className="flex items-center gap-2 text-sm font-semibold text-zinc-600 dark:text-zinc-300">
            {icon}
            {title}
          </h2>
          {action}
        </header>
      )}
      {children}
    </section>
  )
}

type Variant = 'primary' | 'secondary' | 'danger' | 'ghost'

const variants: Record<Variant, string> = {
  primary: 'bg-sky-600 text-white hover:bg-sky-500 disabled:bg-sky-600/50',
  secondary:
    'border border-zinc-300 bg-white text-zinc-800 hover:bg-zinc-50 dark:border-zinc-700 dark:bg-zinc-800 dark:text-zinc-100 dark:hover:bg-zinc-700',
  danger: 'bg-rose-600 text-white hover:bg-rose-500 disabled:bg-rose-600/50',
  ghost: 'text-zinc-600 hover:bg-zinc-100 dark:text-zinc-300 dark:hover:bg-zinc-800',
}

export function Button({
  variant = 'secondary',
  loading,
  icon,
  className,
  children,
  disabled,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant; loading?: boolean; icon?: ReactNode }) {
  return (
    <button
      {...rest}
      disabled={disabled || loading}
      className={cx(
        'inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap rounded-lg px-3 py-1.5 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-60',
        variants[variant],
        className,
      )}
    >
      {loading ? <Loader2 className="size-4 animate-spin" /> : icon}
      {children}
    </button>
  )
}

export function Toggle({
  checked,
  onChange,
  disabled,
  loading,
  label,
}: {
  checked: boolean
  onChange: (v: boolean) => void
  disabled?: boolean
  loading?: boolean
  label?: string
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled || loading}
      onClick={() => onChange(!checked)}
      className={cx(
        'relative inline-flex h-7 w-12 shrink-0 items-center rounded-full transition-colors disabled:opacity-50',
        checked ? 'bg-emerald-500' : 'bg-zinc-300 dark:bg-zinc-700',
      )}
    >
      <span
        className={cx(
          'flex size-5 items-center justify-center rounded-full bg-white shadow transition-transform',
          checked ? 'translate-x-6' : 'translate-x-1',
        )}
      >
        {loading && <Loader2 className="size-3.5 animate-spin text-zinc-500" />}
      </span>
    </button>
  )
}

type Tone = 'ok' | 'warn' | 'bad' | 'neutral' | 'info'

const tones: Record<Tone, string> = {
  ok: 'bg-emerald-100 text-emerald-800 dark:bg-emerald-500/15 dark:text-emerald-300',
  warn: 'bg-amber-100 text-amber-800 dark:bg-amber-500/15 dark:text-amber-300',
  bad: 'bg-rose-100 text-rose-800 dark:bg-rose-500/15 dark:text-rose-300',
  neutral: 'bg-zinc-100 text-zinc-700 dark:bg-zinc-800 dark:text-zinc-300',
  info: 'bg-sky-100 text-sky-800 dark:bg-sky-500/15 dark:text-sky-300',
}

export function Badge({ tone = 'neutral', children }: { tone?: Tone; children: ReactNode }) {
  return (
    <span className={cx('inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium', tones[tone])}>
      {children}
    </span>
  )
}

export function Field({ label, value, mono, copy }: { label: string; value: ReactNode; mono?: boolean; copy?: string | null }) {
  return (
    <div className="flex items-baseline justify-between gap-3 py-1.5 text-sm">
      <span className="shrink-0 text-zinc-500 dark:text-zinc-400">{label}</span>
      <span className={cx('flex min-w-0 items-center gap-1 text-right', mono && 'font-mono text-[13px]')}>
        <span className="truncate">{value ?? '—'}</span>
        {copy && <CopyButton text={copy} />}
      </span>
    </div>
  )
}

export function CopyButton({ text }: { text: string }) {
  const [done, setDone] = useState(false)
  return (
    <button
      type="button"
      title="复制"
      className="rounded p-0.5 text-zinc-400 hover:text-zinc-700 dark:hover:text-zinc-200"
      onClick={() => {
        navigator.clipboard?.writeText(text).then(() => {
          setDone(true)
          setTimeout(() => setDone(false), 1200)
        })
      }}
    >
      {done ? <Check className="size-3.5 text-emerald-500" /> : <Copy className="size-3.5" />}
    </button>
  )
}

export function SignalBars({ level, size = 'md' }: { level: number; size?: 'md' | 'lg' }) {
  const h = size === 'lg' ? [8, 14, 20, 26, 32] : [5, 8, 11, 14, 17]
  const color = level >= 4 ? 'bg-emerald-500' : level >= 2 ? 'bg-amber-500' : 'bg-rose-500'
  return (
    <span className="inline-flex items-end gap-[3px]" aria-label={`信号 ${level}/5`}>
      {h.map((px, i) => (
        <span
          key={i}
          style={{ height: px }}
          className={cx(size === 'lg' ? 'w-2' : 'w-1', 'rounded-sm', i < level ? color : 'bg-zinc-300 dark:bg-zinc-700')}
        />
      ))}
    </span>
  )
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  disabled,
}: {
  value: T | null
  options: { value: T; label: string }[]
  onChange: (v: T) => void
  disabled?: boolean
}) {
  return (
    <div className="inline-flex rounded-lg bg-zinc-100 p-0.5 dark:bg-zinc-800">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          disabled={disabled}
          onClick={() => o.value !== value && onChange(o.value)}
          className={cx(
            'rounded-md px-3 py-1 text-sm font-medium transition-colors disabled:opacity-50',
            o.value === value
              ? 'bg-white text-zinc-900 shadow-sm dark:bg-zinc-600 dark:text-white'
              : 'text-zinc-600 hover:text-zinc-900 dark:text-zinc-400 dark:hover:text-zinc-100',
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  )
}

export function Sparkline({ values, min, max, className }: { values: (number | null)[]; min: number; max: number; className?: string }) {
  const pts = values
    .map((v, i) => (v == null ? null : [i, v] as const))
    .filter((p): p is readonly [number, number] => p !== null)
  if (pts.length < 2) {
    return <div className={cx('flex items-center justify-center text-xs text-zinc-400', className)}>采集中…</div>
  }
  const W = 300
  const H = 60
  const n = Math.max(values.length - 1, 1)
  const y = (v: number) => H - ((Math.min(Math.max(v, min), max) - min) / (max - min)) * H
  const d = pts.map(([i, v], k) => `${k ? 'L' : 'M'}${((i / n) * W).toFixed(1)},${y(v).toFixed(1)}`).join(' ')
  const last = pts[pts.length - 1]
  const area = `${d} L${((last[0] / n) * W).toFixed(1)},${H} L${((pts[0][0] / n) * W).toFixed(1)},${H} Z`
  return (
    <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" className={cx('text-sky-500', className)}>
      <path d={area} fill="currentColor" opacity={0.12} />
      <path d={d} fill="none" stroke="currentColor" strokeWidth={2} vectorEffect="non-scaling-stroke" />
    </svg>
  )
}

export function Empty({ icon, children }: { icon?: ReactNode; children: ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 py-12 text-sm text-zinc-400">
      {icon}
      {children}
    </div>
  )
}
