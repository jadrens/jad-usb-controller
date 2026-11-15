import { BookUser, MessageSquareText, Phone, Plus, Send, Trash2, X } from 'lucide-react'
import { useState } from 'react'
import { api } from '../api'
import { Badge, Button, Card, CopyButton, Empty } from '../components/ui'
import { cx, inputClass } from '../lib/style'
import { useAction, useLive } from '../live-context'

interface Entry {
  dir: 'out' | 'in'
  text: string
  status?: number | null
  local?: boolean
}

interface Preset {
  code: string
  label: string
}

const BUILTIN: Preset[] = [
  { code: '*#06#', label: '查询 IMEI（本地）' },
  { code: '*#100#', label: '查询本机号码' },
  { code: '*100#', label: '余额 / 套餐' },
  { code: '*101#', label: '流量' },
  { code: '*102#', label: '账户信息' },
]

/** SMS query commands of mainland carriers; the reply arrives as a normal SMS. */
const SMS_COMMANDS: { carrier: string; to: string; items: { text: string; label: string }[] }[] = [
  {
    carrier: '中国移动',
    to: '10086',
    items: [
      { text: 'CXLL', label: '查流量' },
      { text: 'CXYE', label: '查余额' },
      { text: 'CXTC', label: '查套餐' },
    ],
  },
  {
    carrier: '中国联通',
    to: '10010',
    items: [
      { text: 'CXLL', label: '查流量' },
      { text: 'CXHF', label: '查话费' },
      { text: 'CXTC', label: '查套餐' },
    ],
  },
  {
    carrier: '中国电信',
    to: '10001',
    items: [
      { text: '108', label: '查流量' },
      { text: '102', label: '查余额' },
    ],
  },
]

const STATUS_LABEL: Record<number, string> = {
  0: '会话结束',
  1: '等待回复',
  2: '网络终止',
  4: '不支持',
  5: '超时',
}

const CUSTOM_KEY = 'usb-card.ussd-presets'

function loadCustom(): Preset[] {
  try {
    return JSON.parse(localStorage.getItem(CUSTOM_KEY) ?? '[]')
  } catch {
    return []
  }
}

function saveCustom(p: Preset[]) {
  try {
    localStorage.setItem(CUSTOM_KEY, JSON.stringify(p))
  } catch {
    /* storage unavailable */
  }
}

export default function Ussd() {
  return (
    <div className="grid gap-4 lg:grid-cols-[1fr_1fr]">
      <div className="space-y-4">
        <OwnNumber />
        <SmsCommands />
      </div>
      <UssdSession />
    </div>
  )
}

function OwnNumber() {
  const { status, connection, setStatus } = useLive()
  const run = useAction()
  const [editing, setEditing] = useState(false)
  const [number, setNumber] = useState('')
  const [busy, setBusy] = useState(false)
  const msisdn = status?.sim.msisdn

  const save = async (value: string) => {
    setBusy(true)
    const s = await run(() => api.setMsisdn(value), value ? '已写入 SIM 卡' : '已清除')
    setBusy(false)
    if (s) {
      setStatus(s)
      setEditing(false)
    }
  }

  return (
    <Card title="本机号码" icon={<BookUser className="size-4" />}>
      <div className="flex items-center gap-2">
        <span className="font-mono text-xl font-semibold">{msisdn ?? <span className="text-base font-normal text-zinc-400">SIM 卡未写入号码</span>}</span>
        {msisdn && <CopyButton text={msisdn} />}
        {!editing && (
          <Button
            variant="ghost"
            className="ml-auto"
            disabled={!connection?.connected || !status?.sim.ready}
            onClick={() => {
              setNumber(msisdn ?? '')
              setEditing(true)
            }}
          >
            {msisdn ? '修改' : '手动写入'}
          </Button>
        )}
      </div>
      {editing && (
        <form
          className="mt-3 flex gap-2"
          onSubmit={(e) => {
            e.preventDefault()
            save(number.trim())
          }}
        >
          <input
            autoFocus
            className={inputClass + ' font-mono'}
            placeholder="+8613800138000"
            value={number}
            onChange={(e) => setNumber(e.target.value.replace(/[^\d+]/g, ''))}
          />
          <Button variant="primary" loading={busy}>
            保存
          </Button>
          {msisdn && (
            <Button variant="danger" disabled={busy} onClick={() => save('')} type="button">
              清除
            </Button>
          )}
          <Button type="button" variant="ghost" icon={<X className="size-4" />} onClick={() => setEditing(false)} />
        </form>
      )}
      <p className="mt-3 text-xs text-zinc-400">
        号码读取自 SIM 卡的本机号码簿（AT+CNUM）。物联卡和多数预付卡不写入号码，可用 USSD「查询本机号码」或给运营商发短信查询后手动写入；部分卡不允许写入。
      </p>
    </Card>
  )
}

function SmsCommands() {
  const { status, connection } = useLive()
  const run = useAction()
  const [sending, setSending] = useState<string | null>(null)
  const carrier = status?.sim.carrier ?? status?.network.operator ?? ''

  const send = async (to: string, text: string) => {
    const key = `${to}:${text}`
    setSending(key)
    await run(() => api.send(to, text), `已发送「${text}」到 ${to}，回复将出现在短信中`)
    setSending(null)
  }

  return (
    <Card title="短信查询指令" icon={<MessageSquareText className="size-4" />}>
      <div className="space-y-3">
        {SMS_COMMANDS.map((g) => (
          <div key={g.to}>
            <div className="mb-1.5 flex items-center gap-2 text-sm">
              <span className="font-medium">{g.carrier}</span>
              <span className="font-mono text-xs text-zinc-400">→ {g.to}</span>
              {carrier.includes(g.carrier.slice(2)) && <Badge tone="info">当前运营商</Badge>}
            </div>
            <div className="flex flex-wrap gap-1.5">
              {g.items.map((it) => (
                <Button
                  key={it.text}
                  className="py-1! text-xs"
                  loading={sending === `${g.to}:${it.text}`}
                  disabled={!connection?.connected}
                  onClick={() => send(g.to, it.text)}
                >
                  {it.label}
                  <span className="font-mono text-zinc-400">{it.text}</span>
                </Button>
              ))}
            </div>
          </div>
        ))}
      </div>
      <p className="mt-3 text-xs text-zinc-400">指令以运营商实际为准；发送普通短信可能产生费用。境外卡请使用 USSD 或运营商提供的方式查询。</p>
    </Card>
  )
}

function UssdSession() {
  const { connection, status } = useLive()
  const run = useAction()
  const [code, setCode] = useState('')
  const [busy, setBusy] = useState(false)
  const [log, setLog] = useState<Entry[]>([])
  const [custom, setCustom] = useState<Preset[]>(loadCustom)
  const [adding, setAdding] = useState(false)
  const [newLabel, setNewLabel] = useState('')
  const waiting = log.length > 0 && log[log.length - 1].status === 1

  const send = async (c: string) => {
    c = c.trim()
    if (!c) return
    setLog((l) => [...l, { dir: 'out', text: c }])
    setCode('')
    // *#06# is answered by the phone itself, never sent to the network
    if (c === '*#06#') {
      setLog((l) => [...l, { dir: 'in', text: `IMEI: ${status?.device.imei ?? '未知'}`, status: 0, local: true }])
      return
    }
    setBusy(true)
    const r = await run(() => api.ussd(c))
    setBusy(false)
    if (r) setLog((l) => [...l, { dir: 'in', text: r.text, status: r.status }])
  }

  const addPreset = () => {
    const c = code.trim()
    if (!c || !newLabel.trim()) return
    const next = [...custom.filter((p) => p.code !== c), { code: c, label: newLabel.trim() }]
    setCustom(next)
    saveCustom(next)
    setAdding(false)
    setNewLabel('')
  }

  const removePreset = (c: string) => {
    const next = custom.filter((p) => p.code !== c)
    setCustom(next)
    saveCustom(next)
  }

  return (
    <div className="space-y-4">
      <Card title="USSD" icon={<Phone className="size-4" />}>
        <form
          className="flex gap-2"
          onSubmit={(e) => {
            e.preventDefault()
            send(code)
          }}
        >
          <input
            className={inputClass + ' font-mono'}
            placeholder={waiting ? '输入菜单选项回复' : '例如 *100#'}
            value={code}
            onChange={(e) => setCode(e.target.value)}
          />
          <Button variant="primary" loading={busy} disabled={!connection?.connected || !code.trim()} icon={<Send className="size-4" />}>
            发送
          </Button>
          {waiting && (
            <Button
              type="button"
              icon={<X className="size-4" />}
              onClick={async () => {
                await run(api.ussdCancel)
                setLog((l) => [...l, { dir: 'in', text: '已结束会话', status: 2 }])
              }}
            >
              结束
            </Button>
          )}
        </form>

        <div className="mt-3 space-y-1.5">
          {[...BUILTIN.map((p) => ({ ...p, custom: false })), ...custom.map((p) => ({ ...p, custom: true }))].map((p) => (
            <div key={p.code} className="group flex items-center gap-2 text-sm">
              <button
                onClick={() => send(p.code)}
                disabled={busy || (!connection?.connected && p.code !== '*#06#')}
                className="flex flex-1 items-center gap-2 rounded-md px-2 py-1 text-left hover:bg-zinc-100 disabled:opacity-50 dark:hover:bg-zinc-800"
              >
                <span className="w-20 shrink-0 font-mono text-sky-600 dark:text-sky-400">{p.code}</span>
                <span className="text-zinc-600 dark:text-zinc-300">{p.label}</span>
                {p.custom && <Badge>自定义</Badge>}
              </button>
              {p.custom && (
                <button className="text-zinc-400 opacity-0 group-hover:opacity-100 hover:text-rose-500" onClick={() => removePreset(p.code)} title="删除">
                  <Trash2 className="size-3.5" />
                </button>
              )}
            </div>
          ))}
        </div>

        {adding ? (
          <div className="mt-2 flex gap-2">
            <input className={inputClass + ' w-28 font-mono'} placeholder="代码" value={code} onChange={(e) => setCode(e.target.value)} />
            <input className={inputClass} placeholder="名称，如：查询余额" value={newLabel} onChange={(e) => setNewLabel(e.target.value)} />
            <Button variant="primary" onClick={addPreset} disabled={!code.trim() || !newLabel.trim()}>
              添加
            </Button>
            <Button variant="ghost" icon={<X className="size-4" />} onClick={() => setAdding(false)} />
          </div>
        ) : (
          <Button variant="ghost" className="mt-2" icon={<Plus className="size-4" />} onClick={() => setAdding(true)}>
            添加常用代码
          </Button>
        )}
        <p className="mt-2 text-xs text-zinc-400">
          USSD 代码因运营商而异，内置项仅为常见写法。很多纯 LTE 模块 / 物联卡不支持 USSD，可改用短信查询。
        </p>
      </Card>

      <Card title="会话记录" action={log.length > 0 && <Button variant="ghost" onClick={() => setLog([])}>清空</Button>}>
        {log.length === 0 ? (
          <Empty>暂无记录</Empty>
        ) : (
          <div className="space-y-2">
            {log.map((e, i) => (
              <div key={i} className={cx('flex flex-col', e.dir === 'out' ? 'items-end' : 'items-start')}>
                <div
                  className={cx(
                    'max-w-[85%] rounded-xl px-3 py-2 text-sm whitespace-pre-wrap',
                    e.dir === 'out' ? 'bg-sky-600 font-mono text-white' : 'bg-zinc-100 dark:bg-zinc-800',
                  )}
                >
                  {e.text}
                </div>
                {e.status != null && (
                  <span className="mt-0.5 text-[11px] text-zinc-400">
                    {e.local ? '本地应答' : (STATUS_LABEL[e.status] ?? `状态 ${e.status}`)}
                  </span>
                )}
              </div>
            ))}
          </div>
        )}
      </Card>
    </div>
  )
}
