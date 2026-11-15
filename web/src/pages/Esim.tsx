import { Check, Layers, Pencil, Power, PowerOff, RefreshCw, Trash2, X } from 'lucide-react'
import { useState } from 'react'
import { api, type EuiccProfile, type EuiccState } from '../api'
import { Badge, Button, Card, CopyButton, Empty, Field } from '../components/ui'
import { formatBytes } from '../lib/format'
import { cx, inputClass } from '../lib/style'
import { useAction, useLive } from '../live-context'

const CLASS_LABEL: Record<string, string> = {
  operational: '运营',
  provisioning: '预置',
  test: '测试',
}

export default function Esim() {
  const { connection, notify } = useLive()
  const run = useAction()
  const [state, setState] = useState<EuiccState | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [busy, setBusy] = useState<string | null>(null)
  const online = !!connection?.connected

  const show = (r: EuiccState) => {
    setState(r)
    // keep the open tab, else start on the eUICC the modem is using
    setSelected((cur) =>
      r.cards.some((c) => c.aid === cur) ? cur : (r.cards.find((c) => c.active) ?? r.cards[0])?.aid ?? null,
    )
  }

  const load = async () => {
    setLoading(true)
    const r = await run(api.euicc)
    setLoading(false)
    if (r) show(r)
  }

  const card = state?.cards.find((c) => c.aid === selected) ?? null

  const act = async (p: EuiccProfile, action: 'enable' | 'disable' | 'delete') => {
    if (!card) return
    const name = profileName(p)
    const prompts = {
      enable: `在 ${card.label} 上启用「${name}」？该芯片上当前启用的 Profile 会被停用，SIM 将重新加载，网络会短暂中断。`,
      disable: `停用「${name}」？${card.active ? '这是模块正在使用的 Profile，停用后将没有可用的 SIM。' : ''}`,
      delete: `永久删除「${name}」（ICCID ${p.iccid}）？\n删除后无法恢复，只能找运营商重新下载。`,
    }
    if (!confirm(prompts[action])) return
    if (action === 'delete' && prompt(`请输入 ICCID 后 4 位确认删除：`) !== p.iccid.slice(-4)) {
      notify('已取消删除', 'info')
      return
    }
    setBusy(`${p.iccid}:${action}`)
    const r = await run(
      () => api.euiccProfile(card.aid, p.iccid, action),
      { enable: '已启用', disable: '已停用', delete: '已删除' }[action],
    )
    setBusy(null)
    if (r) show(r)
  }

  const rename = async (p: EuiccProfile, nickname: string) => {
    if (!card) return
    setBusy(`${p.iccid}:rename`)
    const r = await run(() => api.euiccNickname(card.aid, p.iccid, nickname), '已重命名')
    setBusy(null)
    if (r) show(r)
  }

  const info = card?.info

  return (
    <div className="mx-auto max-w-4xl space-y-4">
      <Card
        title="eUICC / eSIM"
        icon={<Layers className="size-4" />}
        action={
          <Button variant={state ? 'secondary' : 'primary'} icon={<RefreshCw className="size-4" />} loading={loading} disabled={!online} onClick={load}>
            {state ? '刷新' : '读取 eUICC'}
          </Button>
        }
      >
        {!state && (
          <p className="text-sm text-zinc-500">
            检测 SIM 卡上的 eUICC（可写入多个 eSIM Profile 的卡，如 eSTK.me、5ber、eSIM.me、9eSIM 等）并管理其中的 Profile。
            依次尝试标准及各厂商的 ISD-R AID，一张卡上有多个 eUICC（如 eSTK.me 的 SE0 / SE1）时分别显示。
            通过 <code className="font-mono">AT+CSIM</code> 访问，不影响正常使用。
          </p>
        )}
        {state && !state.supported && (
          <div className="text-sm text-zinc-500">
            <Badge tone="neutral">不支持</Badge>
            <span className="ml-2">当前 SIM 卡不是 eUICC（未找到任何已知的 ISD-R）。</span>
          </div>
        )}
        {state && state.cards.length > 0 && (
          <>
            <div className="mb-4 flex gap-2 overflow-x-auto">
              {state.cards.map((c) => (
                <button
                  key={c.aid}
                  onClick={() => setSelected(c.aid)}
                  className={cx(
                    'flex shrink-0 flex-col items-start rounded-lg border px-3 py-2 text-left transition-colors',
                    c.aid === selected
                      ? 'border-sky-500 bg-sky-50 dark:bg-sky-500/10'
                      : 'border-zinc-200 hover:bg-zinc-50 dark:border-zinc-700 dark:hover:bg-zinc-800',
                  )}
                >
                  <span className="flex items-center gap-1.5 text-sm font-medium">
                    {c.label}
                    {c.active && <Badge tone="ok">使用中</Badge>}
                  </span>
                  <span className="font-mono text-[11px] text-zinc-500">
                    EID …{c.info.eid?.slice(-8) ?? '—'} · {c.info.profiles.length} 个 Profile
                  </span>
                </button>
              ))}
            </div>
            {card && info && (
              <div className="grid gap-x-8 sm:grid-cols-2">
                <Field label="EID" value={info.eid} mono copy={info.eid} />
                <Field label="ISD-R AID" value={card.aid} mono />
                <Field label="默认 SM-DP+" value={info.defaultSmdp} mono />
                <Field label="根 SM-DS" value={info.rootSmds} mono />
                <Field label="剩余空间" value={info.freeNvm != null ? formatBytes(info.freeNvm) : null} />
                <Field label="eUICC 固件" value={info.firmware} mono />
                <Field label="SGP.22 版本" value={info.svn} mono />
                {card.aliases.length > 0 && <Field label="同一芯片的其他入口" value={card.aliases.join('、')} />}
              </div>
            )}
          </>
        )}
      </Card>

      {card && info && (
        <Card title={`${card.label} 的 Profile（${info.profiles.length}）`}>
          {info.profiles.length === 0 ? (
            <Empty>该芯片上没有 Profile</Empty>
          ) : (
            <ul className="divide-y divide-zinc-100 dark:divide-zinc-800">
              {info.profiles.map((p) => (
                <ProfileRow key={p.iccid} p={p} busy={busy} disabled={!online || busy !== null} onAction={act} onRename={rename} />
              ))}
            </ul>
          )}
          <p className="mt-3 text-xs text-zinc-400">
            暂不支持下载新 Profile（需要对接运营商 SM-DP+）。启用 / 停用 / 删除后 eUICC 产生的通知不会回传给运营商，一般不影响使用。
            多芯片卡由卡片自身决定哪个芯片的 Profile 对模块生效（如 eSTK.me 的 STK 菜单设置）。
          </p>
        </Card>
      )}
    </div>
  )
}

function profileName(p: EuiccProfile) {
  return p.nickname || p.name || p.provider || p.iccid
}

function ProfileRow({
  p,
  busy,
  disabled,
  onAction,
  onRename,
}: {
  p: EuiccProfile
  busy: string | null
  disabled: boolean
  onAction: (p: EuiccProfile, a: 'enable' | 'disable' | 'delete') => void
  onRename: (p: EuiccProfile, nickname: string) => Promise<void>
}) {
  const [editing, setEditing] = useState(false)
  const [nick, setNick] = useState('')
  const is = (a: string) => busy === `${p.iccid}:${a}`

  return (
    <li className="flex flex-wrap items-center gap-3 py-3">
      {p.icon ? (
        <img src={p.icon} alt="" className="size-9 shrink-0 rounded-lg bg-white object-contain p-0.5" />
      ) : (
        <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-zinc-100 text-sm font-semibold text-zinc-500 dark:bg-zinc-800">
          {profileName(p).slice(0, 1)}
        </span>
      )}
      <div className="min-w-0 flex-1">
        {editing ? (
          <form
            className="flex gap-2"
            onSubmit={async (e) => {
              e.preventDefault()
              await onRename(p, nick.trim())
              setEditing(false)
            }}
          >
            <input autoFocus className={inputClass} value={nick} maxLength={32} onChange={(e) => setNick(e.target.value)} placeholder="昵称" />
            <Button variant="primary" loading={is('rename')} icon={<Check className="size-4" />} />
            <Button type="button" variant="ghost" icon={<X className="size-4" />} onClick={() => setEditing(false)} />
          </form>
        ) : (
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium">{profileName(p)}</span>
            {p.enabled ? <Badge tone="ok">已启用</Badge> : <Badge>未启用</Badge>}
            {p.class && p.class !== 'operational' && <Badge tone="warn">{CLASS_LABEL[p.class]}</Badge>}
            <button
              className="text-zinc-400 hover:text-zinc-700 dark:hover:text-zinc-200"
              title="重命名"
              disabled={disabled}
              onClick={() => {
                setNick(p.nickname ?? '')
                setEditing(true)
              }}
            >
              <Pencil className="size-3.5" />
            </button>
          </div>
        )}
        <div className="mt-0.5 flex items-center gap-1 font-mono text-xs text-zinc-500">
          {p.iccid}
          <CopyButton text={p.iccid} />
          {p.provider && p.provider !== profileName(p) && <span className="ml-2 font-sans">{p.provider}</span>}
        </div>
      </div>
      <div className="flex gap-2">
        {p.enabled ? (
          <Button icon={<PowerOff className="size-4" />} loading={is('disable')} disabled={disabled} onClick={() => onAction(p, 'disable')}>
            停用
          </Button>
        ) : (
          <>
            <Button variant="primary" icon={<Power className="size-4" />} loading={is('enable')} disabled={disabled} onClick={() => onAction(p, 'enable')}>
              启用
            </Button>
            <Button variant="ghost" className="text-rose-600!" icon={<Trash2 className="size-4" />} loading={is('delete')} disabled={disabled} onClick={() => onAction(p, 'delete')}>
              删除
            </Button>
          </>
        )}
      </div>
    </li>
  )
}
