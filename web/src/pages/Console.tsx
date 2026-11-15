import { Pause, Play, SquareTerminal, Trash2 } from 'lucide-react'
import { useLayoutEffect, useMemo, useRef, useState } from 'react'
import { api } from '../api'
import { Button, Card } from '../components/ui'
import { cx, inputClass } from '../lib/style'
import { useAction, useLive } from '../live-context'

const QUICK = [
  'ATI',
  'AT+CSQ',
  'AT+CPIN?',
  'AT+COPS?',
  'AT+CEREG?',
  'AT+CGDCONT?',
  'AT+CGPADDR',
  'AT+QENG="servingcell"',
  'AT+QNWINFO',
  'AT+QCFG="usbnet"',
  'AT+CPMS?',
]

interface Exchange {
  id: number
  command: string
  lines: string[]
  ok: boolean | null
  error?: string | null
  ms?: number
}

const HISTORY_KEY = 'usb-card.at-history'

function loadHistory(): string[] {
  try {
    return JSON.parse(localStorage.getItem(HISTORY_KEY) ?? '[]')
  } catch {
    return []
  }
}

export default function Console() {
  const { connection, atLog, clearAtLog } = useLive()
  const run = useAction()
  const [cmd, setCmd] = useState('')
  const [timeout, setTimeoutMs] = useState(5000)
  const [session, setSession] = useState<Exchange[]>([])
  const [history, setHistory] = useState<string[]>(loadHistory)
  const [hIdx, setHIdx] = useState(-1)
  const [frozen, setFrozen] = useState<typeof atLog | null>(null)
  const [filter, setFilter] = useState('')
  const [onlyUrc, setOnlyUrc] = useState(false)
  const nextId = useRef(1)
  const sessionEnd = useRef<HTMLDivElement>(null)
  const logBox = useRef<HTMLDivElement>(null)

  const paused = frozen !== null
  const shown = frozen ?? atLog

  const visible = useMemo(() => {
    const f = filter.trim().toUpperCase()
    return shown.filter((e) => (!onlyUrc || e.dir === 'urc') && (!f || e.line.toUpperCase().includes(f))).slice(-600)
  }, [shown, filter, onlyUrc])

  useLayoutEffect(() => {
    sessionEnd.current?.scrollIntoView({ block: 'end' })
  }, [session])

  useLayoutEffect(() => {
    const el = logBox.current
    if (el) el.scrollTop = el.scrollHeight
  }, [visible])

  const exec = async (command: string) => {
    command = command.trim()
    if (!command) return
    const id = nextId.current++
    setSession((s) => [...s.slice(-100), { id, command, lines: [], ok: null }])
    const h = [command, ...history.filter((x) => x !== command)].slice(0, 50)
    setHistory(h)
    try {
      localStorage.setItem(HISTORY_KEY, JSON.stringify(h))
    } catch {
      /* storage unavailable */
    }
    setHIdx(-1)
    setCmd('')
    const t0 = performance.now()
    const r = await run(() => api.at(command, timeout))
    const ms = Math.round(performance.now() - t0)
    setSession((s) =>
      s.map((x) =>
        x.id === id ? (r ? { ...x, lines: r.lines, ok: r.ok, error: r.error, ms } : { ...x, ok: false, error: '请求失败', ms }) : x,
      ),
    )
  }

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Card title="AT 命令" icon={<SquareTerminal className="size-4" />} action={<Button variant="ghost" onClick={() => setSession([])}>清屏</Button>}>
        <div className="h-[55dvh] overflow-y-auto rounded-lg bg-zinc-950 p-3 font-mono text-[13px] leading-relaxed text-zinc-200">
          {session.length === 0 && <div className="text-zinc-500">输入 AT 命令并回车执行。↑/↓ 浏览历史。</div>}
          {session.map((x) => (
            <div key={x.id} className="mb-2">
              <div className="text-sky-400">
                <span className="text-zinc-500">›</span> {x.command}
                {x.ms != null && <span className="ml-2 text-[11px] text-zinc-600">{x.ms} ms</span>}
              </div>
              {x.lines.map((l, i) => (
                <div key={i} className="break-all whitespace-pre-wrap">{l}</div>
              ))}
              {x.ok === null && <div className="text-zinc-500">…</div>}
              {x.ok === true && <div className="text-emerald-400">OK</div>}
              {x.ok === false && <div className="text-rose-400">{x.error ?? 'ERROR'}</div>}
            </div>
          ))}
          <div ref={sessionEnd} />
        </div>
        <form
          className="mt-3 flex gap-2"
          onSubmit={(e) => {
            e.preventDefault()
            exec(cmd)
          }}
        >
          <input
            className={inputClass + ' font-mono'}
            placeholder={connection?.connected ? 'AT+CSQ' : '模块离线'}
            value={cmd}
            spellCheck={false}
            autoCapitalize="off"
            onChange={(e) => setCmd(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'ArrowUp' && history.length) {
                e.preventDefault()
                const i = Math.min(hIdx + 1, history.length - 1)
                setHIdx(i)
                setCmd(history[i])
              } else if (e.key === 'ArrowDown') {
                e.preventDefault()
                const i = hIdx - 1
                setHIdx(Math.max(i, -1))
                setCmd(i >= 0 ? history[i] : '')
              }
            }}
          />
          <select
            className={inputClass + ' w-auto'}
            value={timeout}
            onChange={(e) => setTimeoutMs(Number(e.target.value))}
            title="超时"
          >
            {[2000, 5000, 15000, 60000, 180000].map((t) => (
              <option key={t} value={t}>
                {t / 1000}s
              </option>
            ))}
          </select>
          <Button variant="primary" disabled={!connection?.connected || !cmd.trim()}>
            执行
          </Button>
        </form>
        <div className="mt-2 flex flex-wrap gap-1.5">
          {QUICK.map((q) => (
            <button
              key={q}
              disabled={!connection?.connected}
              onClick={() => exec(q)}
              className="rounded-md bg-zinc-100 px-2 py-0.5 font-mono text-xs text-zinc-600 hover:bg-zinc-200 disabled:opacity-50 dark:bg-zinc-800 dark:text-zinc-300 dark:hover:bg-zinc-700"
            >
              {q}
            </button>
          ))}
        </div>
      </Card>

      <Card
        title="串口实时日志"
        action={
          <div className="flex items-center gap-1">
            <label className="mr-1 flex items-center gap-1 text-xs text-zinc-500">
              <input type="checkbox" checked={onlyUrc} onChange={(e) => setOnlyUrc(e.target.checked)} />
              仅 URC
            </label>
            <Button variant="ghost" icon={paused ? <Play className="size-4" /> : <Pause className="size-4" />} onClick={() => setFrozen(paused ? null : atLog)} title={paused ? '继续' : '暂停'} />
            <Button variant="ghost" icon={<Trash2 className="size-4" />} onClick={clearAtLog} title="清空" />
          </div>
        }
      >
        <input className={inputClass + ' mb-2 font-mono'} placeholder="过滤…" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <div ref={logBox} className="h-[55dvh] overflow-y-auto rounded-lg bg-zinc-950 p-3 font-mono text-[12px] leading-relaxed">
          {visible.map((e, i) => (
            <div key={i} className="flex gap-2">
              <span className="shrink-0 text-zinc-600">{e.ts.slice(11, 19)}</span>
              <span
                className={cx(
                  'w-8 shrink-0 font-semibold',
                  e.dir === 'tx' ? 'text-sky-400' : e.dir === 'urc' ? 'text-amber-400' : 'text-emerald-400',
                )}
              >
                {e.dir.toUpperCase()}
              </span>
              <span className="break-all text-zinc-300">{e.line}</span>
            </div>
          ))}
        </div>
      </Card>
    </div>
  )
}
