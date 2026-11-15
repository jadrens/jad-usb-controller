import { ArrowDown, ArrowUp, History, Pause, Play, RadioTower, SignalHigh, Waves } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { api, type Carrier, type CellInfo, type ServingCell } from '../api'
import { Badge, Button, Card, Empty, Field, Sparkline } from '../components/ui'
import { cx } from '../lib/style'
import { useLive } from '../live-context'

const POLL_MS = 5000
const MAX_SAMPLES = 120

const STATE_LABEL: Record<string, { text: string; tone: 'ok' | 'warn' | 'bad' | 'info' }> = {
  CONNECT: { text: '连接态（有数据）', tone: 'ok' },
  NOCONN: { text: '空闲态（已驻留）', tone: 'info' },
  LIMSRV: { text: '受限服务', tone: 'warn' },
  SEARCH: { text: '搜索中', tone: 'bad' },
}

const SCAN_MODE: Record<string, string> = {
  '0': '自动',
  '1': '仅 GSM',
  '2': '仅 WCDMA',
  '3': '仅 LTE',
}

type Quality = 'great' | 'good' | 'fair' | 'poor' | 'bad'

const QUALITY_CLASS: Record<Quality, string> = {
  great: 'text-emerald-600 dark:text-emerald-400',
  good: 'text-lime-600 dark:text-lime-400',
  fair: 'text-amber-600 dark:text-amber-400',
  poor: 'text-orange-600 dark:text-orange-400',
  bad: 'text-rose-600 dark:text-rose-400',
}

const QUALITY_BAR: Record<Quality, string> = {
  great: 'bg-emerald-500',
  good: 'bg-lime-500',
  fair: 'bg-amber-500',
  poor: 'bg-orange-500',
  bad: 'bg-rose-500',
}

function grade(v: number | null, steps: [number, number, number, number]): Quality | null {
  if (v == null) return null
  if (v >= steps[0]) return 'great'
  if (v >= steps[1]) return 'good'
  if (v >= steps[2]) return 'fair'
  if (v >= steps[3]) return 'poor'
  return 'bad'
}

const rsrpGrade = (v: number | null) => grade(v, [-80, -90, -100, -110])
const rsrqGrade = (v: number | null) => grade(v, [-8, -11, -15, -19])
const sinrGrade = (v: number | null) => grade(v, [20, 13, 5, 0])

interface Sample {
  t: number
  rsrp: number | null
  sinr: number | null
}

interface Handover {
  t: number
  from: string
  to: string
}

// survives switching tabs during the session
const session: { samples: Sample[]; handovers: Handover[]; lastKey: string | null } = {
  samples: [],
  handovers: [],
  lastKey: null,
}

function cellKey(s: ServingCell | null) {
  if (!s?.carrier) return null
  return `B${s.carrier.band ?? '?'} · ${s.carrier.earfcn} · PCI ${s.pci ?? '?'}`
}

function freq(c: Carrier | null) {
  return c?.dlMhz != null ? `${c.dlMhz.toFixed(1)} MHz` : '—'
}

export default function Cell() {
  const { connection } = useLive()
  const [info, setInfo] = useState<CellInfo | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [paused, setPaused] = useState(false)
  const [, force] = useState(0)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const online = !!connection?.connected

  const poll = useCallback(async () => {
    try {
      const r = await api.cell()
      setInfo(r)
      setError(null)
      const s = r.serving
      session.samples = [...session.samples, { t: Date.now(), rsrp: s?.rsrp ?? null, sinr: s?.sinr ?? null }].slice(-MAX_SAMPLES)
      const key = cellKey(s)
      if (key && session.lastKey && key !== session.lastKey) {
        session.handovers = [{ t: Date.now(), from: session.lastKey, to: key }, ...session.handovers].slice(0, 50)
      }
      if (key) session.lastKey = key
      force((n) => n + 1)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }, [])

  useEffect(() => {
    if (!online || paused) return
    let stopped = false
    const loop = async () => {
      await poll()
      if (!stopped) timer.current = setTimeout(loop, POLL_MS)
    }
    void loop()
    return () => {
      stopped = true
      clearTimeout(timer.current)
    }
  }, [online, paused, poll])

  const s = info?.serving
  const state = s ? STATE_LABEL[s.state] : null
  const currentBand = s?.carrier?.band

  return (
    <div className="space-y-4">
      <Card
        title="服务小区"
        icon={<RadioTower className="size-4" />}
        action={
          <div className="flex items-center gap-2">
            <span className="text-xs text-zinc-400">{paused ? '已暂停' : `每 ${POLL_MS / 1000} 秒刷新`}</span>
            <Button
              variant="ghost"
              icon={paused ? <Play className="size-4" /> : <Pause className="size-4" />}
              onClick={() => setPaused(!paused)}
              title={paused ? '继续' : '暂停'}
            />
          </div>
        }
      >
        {error && <p className="mb-2 text-sm text-rose-500">{error}</p>}
        {!info ? (
          <Empty>{online ? '读取中…' : '模块离线'}</Empty>
        ) : !s ? (
          <Empty>模块未返回小区信息（需要 Quectel 模块）</Empty>
        ) : s.rat !== 'LTE' ? (
          <div className="space-y-2 text-sm">
            <div className="flex items-center gap-2">
              <Badge tone="info">{s.rat || '未知制式'}</Badge>
              {state ? <Badge tone={state.tone}>{state.text}</Badge> : <Badge>{s.state}</Badge>}
            </div>
            {s.raw.length > 0 && <p className="font-mono text-xs text-zinc-500">{s.raw.join(', ')}</p>}
          </div>
        ) : (
          <div className="grid gap-6 lg:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)]">
            <div>
              <div className="flex flex-wrap items-end gap-x-4 gap-y-2">
                <span className="rounded-xl bg-sky-600 px-3 py-1 text-3xl font-bold text-white tabular-nums">
                  B{s.carrier?.band ?? '?'}
                </span>
                <div>
                  <div className="text-lg font-semibold">
                    LTE {s.carrier?.duplex ?? s.duplex}
                    {s.dlBandwidthMhz != null && <span className="ml-2 text-zinc-500">{s.dlBandwidthMhz} MHz 带宽</span>}
                  </div>
                  <div className="flex gap-3 text-sm text-zinc-500 tabular-nums">
                    <span className="flex items-center gap-1">
                      <ArrowDown className="size-3.5 text-sky-500" />
                      {freq(s.carrier)}
                    </span>
                    {s.carrier?.ulMhz != null && s.carrier.duplex === 'FDD' && (
                      <span className="flex items-center gap-1">
                        <ArrowUp className="size-3.5 text-emerald-500" />
                        {s.carrier.ulMhz.toFixed(1)} MHz
                      </span>
                    )}
                  </div>
                </div>
                {state ? <Badge tone={state.tone}>{state.text}</Badge> : <Badge>{s.state}</Badge>}
              </div>

              <div className="mt-4 grid grid-cols-2 gap-2 sm:grid-cols-4">
                <Metric label="RSRP" value={s.rsrp} unit="dBm" q={rsrpGrade(s.rsrp)} />
                <Metric label="RSRQ" value={s.rsrq} unit="dB" q={rsrqGrade(s.rsrq)} />
                <Metric label="SINR" value={s.sinr} unit="dB" q={sinrGrade(s.sinr)} />
                <Metric label="RSSI" value={s.rssi} unit="dBm" q={null} />
              </div>
            </div>

            <div className="grid grid-cols-1 gap-x-6 sm:grid-cols-2 lg:grid-cols-1 xl:grid-cols-2">
              <Field label="EARFCN" value={s.carrier?.earfcn} mono />
              <Field label="PCI" value={s.pci} mono />
              <Field label="小区 ID" value={s.cellId} mono copy={s.cellId} />
              <Field label="基站 / 扇区" value={s.enbId != null ? `${s.enbId} / ${s.sector}` : null} mono />
              <Field label="TAC" value={s.tac} mono />
              <Field label="MCC-MNC" value={s.mcc ? `${s.mcc}-${s.mnc}` : null} mono />
              <Field label="上行带宽" value={s.ulBandwidthMhz != null ? `${s.ulBandwidthMhz} MHz` : null} />
              <Field label="Srxlev" value={s.srxlev} mono />
            </div>
          </div>
        )}
      </Card>

      <div className="grid gap-4 lg:grid-cols-2">
        <Card title="RSRP 趋势" icon={<SignalHigh className="size-4" />} action={<span className="text-xs text-zinc-400">dBm · 本次会话</span>}>
          <Sparkline className="h-20 w-full" values={session.samples.map((x) => x.rsrp)} min={-130} max={-70} />
        </Card>
        <Card title="SINR 趋势" icon={<Waves className="size-4" />} action={<span className="text-xs text-zinc-400">dB · 本次会话</span>}>
          <Sparkline className="h-20 w-full text-emerald-500" values={session.samples.map((x) => x.sinr)} min={-10} max={30} />
        </Card>
      </div>

      <Card title={`邻区（${info?.neighbours.length ?? 0}）`} icon={<RadioTower className="size-4" />}>
        {!info?.neighbours.length ? (
          <Empty>没有测量到邻区</Empty>
        ) : (
          <div className="-mx-4 overflow-x-auto px-4">
            <table className="w-full min-w-[560px] text-left text-sm">
              <thead className="text-xs text-zinc-500">
                <tr className="border-b border-zinc-100 dark:border-zinc-800">
                  <th className="py-2 font-medium">类型</th>
                  <th className="font-medium">频段</th>
                  <th className="font-medium">EARFCN</th>
                  <th className="font-medium">下行频率</th>
                  <th className="font-medium">PCI</th>
                  <th className="w-48 font-medium">RSRP</th>
                  <th className="font-medium">RSRQ</th>
                </tr>
              </thead>
              <tbody className="tabular-nums">
                {info.neighbours.map((n) => {
                  const q = rsrpGrade(n.rsrp)
                  return (
                    <tr key={`${n.carrier.earfcn}-${n.pci}`} className="border-b border-zinc-50 dark:border-zinc-800/50">
                      <td className="py-1.5">
                        <Badge tone={n.kind === 'intra' ? 'info' : 'neutral'}>{n.kind === 'intra' ? '同频' : '异频'}</Badge>
                      </td>
                      <td className={cx('font-semibold', n.carrier.band === currentBand && 'text-sky-600 dark:text-sky-400')}>
                        B{n.carrier.band ?? '?'}
                        <span className="ml-1 text-xs font-normal text-zinc-400">{n.carrier.duplex}</span>
                      </td>
                      <td className="font-mono">{n.carrier.earfcn}</td>
                      <td>{freq(n.carrier)}</td>
                      <td className="font-mono">{n.pci}</td>
                      <td>
                        <div className="flex items-center gap-2">
                          <div className="h-1.5 w-24 overflow-hidden rounded bg-zinc-200 dark:bg-zinc-700">
                            {n.rsrp != null && q && (
                              <div className={cx('h-full', QUALITY_BAR[q])} style={{ width: `${Math.max(4, Math.min(100, ((n.rsrp + 130) / 60) * 100))}%` }} />
                            )}
                          </div>
                          <span className={cx(q && QUALITY_CLASS[q])}>{n.rsrp ?? '—'}</span>
                        </div>
                      </td>
                      <td className={cx(rsrqGrade(n.rsrq) && QUALITY_CLASS[rsrqGrade(n.rsrq)!])}>{n.rsrq ?? '—'}</td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          </div>
        )}
        {!!info?.configured.length && (
          <div className="mt-3">
            <div className="mb-1.5 text-xs text-zinc-500">网络下发、尚未测到信号的频点</div>
            <div className="flex flex-wrap gap-1.5">
              {info.configured.map((c) => (
                <span key={c.earfcn} className="rounded-md bg-zinc-100 px-2 py-0.5 text-xs text-zinc-600 tabular-nums dark:bg-zinc-800 dark:text-zinc-300">
                  B{c.band ?? '?'} · {c.earfcn} · {freq(c)}
                </span>
              ))}
            </div>
          </div>
        )}
        {!!info?.otherRat.length && (
          <p className="mt-3 font-mono text-xs text-zinc-400">其他制式：{info.otherRat.join('；')}</p>
        )}
      </Card>

      <div className="grid gap-4 lg:grid-cols-2">
        <Card title="模块允许的 LTE 频段" icon={<Waves className="size-4" />}>
          {info?.lteBands.length ? (
            <div className="flex flex-wrap gap-1.5">
              {info.lteBands.map((b) => (
                <span
                  key={b}
                  className={cx(
                    'rounded-md px-2 py-0.5 text-sm font-medium tabular-nums',
                    b === currentBand
                      ? 'bg-sky-600 text-white'
                      : info.neighbours.some((n) => n.carrier.band === b)
                        ? 'bg-sky-100 text-sky-800 dark:bg-sky-500/15 dark:text-sky-300'
                        : 'bg-zinc-100 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-300',
                  )}
                >
                  B{b}
                </span>
              ))}
            </div>
          ) : (
            <p className="text-sm text-zinc-400">—</p>
          )}
          <p className="mt-3 text-xs text-zinc-400">
            深色为当前服务频段，浅蓝为有邻区的频段。网络搜索模式：{SCAN_MODE[info?.scanMode ?? ''] ?? info?.scanMode ?? '—'}
          </p>
        </Card>

        <Card
          title="小区切换记录"
          icon={<History className="size-4" />}
          action={
            session.handovers.length > 0 && (
              <Button
                variant="ghost"
                onClick={() => {
                  session.handovers = []
                  force((n) => n + 1)
                }}
              >
                清空
              </Button>
            )
          }
        >
          {session.handovers.length === 0 ? (
            <p className="text-sm text-zinc-400">本次打开页面后未发生切换{session.lastKey && `，当前 ${session.lastKey}`}</p>
          ) : (
            <ul className="max-h-48 space-y-1 overflow-y-auto text-sm tabular-nums">
              {session.handovers.map((h) => (
                <li key={h.t} className="flex flex-wrap gap-x-2">
                  <span className="text-zinc-400">{new Date(h.t).toLocaleTimeString('zh-CN')}</span>
                  <span className="text-zinc-500">{h.from}</span>
                  <span>→</span>
                  <span className="font-medium">{h.to}</span>
                </li>
              ))}
            </ul>
          )}
        </Card>
      </div>
    </div>
  )
}

function Metric({ label, value, unit, q }: { label: string; value: number | null; unit: string; q: Quality | null }) {
  return (
    <div className="rounded-lg bg-zinc-50 px-3 py-2 dark:bg-zinc-800/60">
      <div className="text-xs text-zinc-500">{label}</div>
      <div className={cx('text-xl font-semibold tabular-nums', q && QUALITY_CLASS[q])}>
        {value ?? '—'}
        {value != null && <span className="ml-0.5 text-xs font-normal text-zinc-400">{unit}</span>}
      </div>
    </div>
  )
}
