import { Archive, HardDrive, Network, RefreshCw, Server, Usb } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { api, type DeviceListing, type PdpContext, type SmsSettings } from '../api'
import { Badge, Button, Card, Field, Segmented, Toggle } from '../components/ui'
import { STORAGE_LABEL } from '../lib/format'
import { inputClass } from '../lib/style'
import { useAction, useLive } from '../live-context'

const hex = (n: number | null) => (n == null ? '—' : n.toString(16).padStart(4, '0'))

const APN_PRESETS = [
  { label: '移动物联网 cmiot', apn: 'cmiot' },
  { label: '移动 cmnet', apn: 'cmnet' },
  { label: '联通物联网 scuiot', apn: 'scuiot' },
  { label: '联通 3gnet', apn: '3gnet' },
  { label: '电信物联网 ctnet', apn: 'ctnet' },
]

export default function Settings() {
  const { connection, config, status } = useLive()
  const run = useAction()
  const online = !!connection?.connected

  const [contexts, setContexts] = useState<PdpContext[] | null>(null)
  const [cid, setCid] = useState(config?.dataCid ?? 1)
  const [pdpType, setPdpType] = useState('IP')
  const [apn, setApn] = useState('')
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [auth, setAuth] = useState(1)
  const [saving, setSaving] = useState(false)
  const [devices, setDevices] = useState<DeviceListing | null>(null)
  const [scanning, setScanning] = useState(false)

  const loadApn = useCallback(
    () =>
      api
        .apn()
        .catch(() => null)
        .then((list) => {
          setContexts(list)
          const cur = list?.find((c) => c.cid === (config?.dataCid ?? 1))
          if (cur) {
            setPdpType(cur.pdpType || 'IP')
            setApn(cur.apn)
          }
        }),
    [config?.dataCid],
  )

  const fetchDevices = useCallback(
    () =>
      api
        .devices()
        .catch(() => null)
        .then(setDevices),
    [],
  )

  const scan = async () => {
    setScanning(true)
    await fetchDevices()
    setScanning(false)
  }

  useEffect(() => {
    if (online) void loadApn()
  }, [online, loadApn])

  useEffect(() => {
    void fetchDevices()
  }, [fetchDevices])

  const save = async () => {
    setSaving(true)
    const r = await run(
      () => api.setApn({ cid, pdpType, apn: apn.trim(), username: username || undefined, password: password || undefined, auth }),
      'APN 已保存，重新开启移动数据后生效',
    )
    setSaving(false)
    if (r) setContexts(r)
  }

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Card title="APN 设置" icon={<Network className="size-4" />}>
        <div className="grid grid-cols-[auto_1fr] items-center gap-x-3 gap-y-2.5 text-sm">
          <label className="text-zinc-500">CID</label>
          <input
            type="number"
            min={1}
            max={15}
            className={inputClass + ' w-24'}
            value={cid}
            onChange={(e) => setCid(Number(e.target.value))}
          />
          <label className="text-zinc-500">类型</label>
          <select className={inputClass} value={pdpType} onChange={(e) => setPdpType(e.target.value)}>
            <option value="IP">IPv4</option>
            <option value="IPV6">IPv6</option>
            <option value="IPV4V6">IPv4v6</option>
          </select>
          <label className="text-zinc-500">APN</label>
          <div>
            <input className={inputClass + ' font-mono'} value={apn} onChange={(e) => setApn(e.target.value)} placeholder="cmiot" />
            <div className="mt-1.5 flex flex-wrap gap-1">
              {APN_PRESETS.map((p) => (
                <button
                  key={p.apn}
                  onClick={() => setApn(p.apn)}
                  className="rounded bg-zinc-100 px-1.5 py-0.5 text-[11px] text-zinc-600 hover:bg-zinc-200 dark:bg-zinc-800 dark:text-zinc-300"
                >
                  {p.label}
                </button>
              ))}
            </div>
          </div>
          <label className="text-zinc-500">用户名</label>
          <input className={inputClass} value={username} onChange={(e) => setUsername(e.target.value)} placeholder="可选（专网卡）" />
          <label className="text-zinc-500">密码</label>
          <input type="password" className={inputClass} value={password} onChange={(e) => setPassword(e.target.value)} placeholder="可选" />
          <label className="text-zinc-500">认证</label>
          <select className={inputClass} value={auth} onChange={(e) => setAuth(Number(e.target.value))} disabled={!username}>
            <option value={0}>无</option>
            <option value={1}>PAP</option>
            <option value={2}>CHAP</option>
          </select>
        </div>
        <div className="mt-4 flex gap-2">
          <Button variant="primary" loading={saving} disabled={!online || !apn.trim()} onClick={save}>
            保存
          </Button>
          <Button disabled={!online} onClick={loadApn} icon={<RefreshCw className="size-4" />}>
            读取
          </Button>
        </div>
        {contexts && contexts.length > 0 && (
          <table className="mt-4 w-full text-left text-xs">
            <thead className="text-zinc-500">
              <tr>
                <th className="py-1 font-medium">CID</th>
                <th className="font-medium">类型</th>
                <th className="font-medium">APN</th>
                <th className="font-medium">地址</th>
              </tr>
            </thead>
            <tbody className="font-mono">
              {contexts.map((c) => (
                <tr key={c.cid} className="border-t border-zinc-100 dark:border-zinc-800">
                  <td className="py-1">{c.cid}</td>
                  <td>{c.pdpType}</td>
                  <td>{c.apn || '—'}</td>
                  <td>{c.address ?? '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>

      <div className="space-y-4">
        <SmsStorageCard />

        <Card title="服务" icon={<Server className="size-4" />}>
          <Field label="连接状态" value={<Badge tone={online ? 'ok' : 'bad'}>{online ? '已连接' : '未连接'}</Badge>} />
          <Field label="通道" value={connection?.transport} />
          <Field label="传输模式" value={config?.transport} />
          <Field label="轮询间隔" value={config ? `${config.pollSecs} 秒` : null} />
          <Field label="数据 CID" value={config?.dataCid} />
          <Field label="模块 USB 网卡模式" value={status?.device.usbNetMode} />
          {connection?.error && <p className="mt-2 rounded-lg bg-rose-50 p-2 text-xs break-all text-rose-700 dark:bg-rose-950/40 dark:text-rose-300">{connection.error}</p>}
        </Card>

        <Card
          title="检测到的设备"
          icon={<Usb className="size-4" />}
          action={<Button variant="ghost" icon={<RefreshCw className="size-4" />} loading={scanning} onClick={scan} />}
        >
          {devices?.usb.length === 0 && devices.serial.length === 0 && <p className="text-sm text-zinc-400">未发现 USB 模块</p>}
          <ul className="space-y-3">
            {devices?.usb.map((d) => (
              <li key={`${d.bus}-${d.address}`} className="rounded-lg border border-zinc-200 p-3 text-sm dark:border-zinc-800">
                <div className="flex items-center gap-2">
                  <Usb className="size-4 text-sky-500" />
                  <span className="font-medium">{d.knownVendor ?? d.manufacturer}</span>
                  <span className="font-mono text-xs text-zinc-500">
                    {hex(d.vid)}:{hex(d.pid)}
                  </span>
                  <span className="ml-auto text-xs text-zinc-400">
                    bus {d.bus} · dev {d.address}
                  </span>
                </div>
                <ul className="mt-2 space-y-0.5 font-mono text-xs text-zinc-500">
                  {d.interfaces.map((i) => (
                    <li key={i.number}>
                      #{i.number} class 0x{i.class.toString(16).padStart(2, '0')} {i.name ?? ''}
                    </li>
                  ))}
                </ul>
              </li>
            ))}
            {devices?.serial.map((s) => (
              <li key={s.path} className="flex items-center gap-2 rounded-lg border border-zinc-200 p-3 text-sm dark:border-zinc-800">
                <HardDrive className="size-4 text-zinc-500" />
                <span className="font-mono">{s.path}</span>
                <span className="font-mono text-xs text-zinc-500">
                  {hex(s.vid)}:{hex(s.pid)}
                </span>
                <span className="ml-auto text-xs text-zinc-400">{s.knownVendor ?? s.product}</span>
              </li>
            ))}
          </ul>
          <p className="mt-3 text-xs text-zinc-400">
            直接 USB 访问需要设备读写权限：请安装 <code className="font-mono">deploy/70-usb-card.rules</code> udev 规则，或以 root 运行服务。
          </p>
        </Card>
      </div>
    </div>
  )
}

function SmsStorageCard() {
  const { connection, smsVersion } = useLive()
  const run = useAction()
  const [settings, setSettings] = useState<SmsSettings | null>(null)
  const [busy, setBusy] = useState(false)
  const online = !!connection?.connected

  useEffect(() => {
    api
      .smsSettings()
      .then(setSettings)
      .catch(() => {})
  }, [smsVersion, online])

  const update = async (body: Parameters<typeof api.setSmsSettings>[0], msg: string) => {
    setBusy(true)
    const r = await run(() => api.setSmsSettings(body), msg)
    setBusy(false)
    if (r) setSettings(r)
  }

  return (
    <Card title="短信存储" icon={<Archive className="size-4" />}>
      {settings?.usage.length ? (
        <div className="mb-3 grid grid-cols-2 gap-2">
          {settings.usage.map((u) => (
            <div key={u.memory} className="rounded-lg bg-zinc-50 p-2.5 dark:bg-zinc-800/60">
              <div className="flex items-center justify-between text-xs text-zinc-500">
                <span>{STORAGE_LABEL[u.memory] ?? u.memory}</span>
                <span className="font-mono">{u.memory}</span>
              </div>
              <div className="mt-1 text-lg font-semibold tabular-nums">
                {u.used}
                <span className="text-sm font-normal text-zinc-400"> / {u.total}</span>
              </div>
              <div className="mt-1 h-1 overflow-hidden rounded bg-zinc-200 dark:bg-zinc-700">
                <div className="h-full bg-sky-500" style={{ width: `${Math.min(100, (u.used / Math.max(u.total, 1)) * 100)}%` }} />
              </div>
            </div>
          ))}
        </div>
      ) : (
        <p className="mb-3 text-sm text-zinc-400">同步短信后显示各存储的用量</p>
      )}
      <div className="flex items-center justify-between gap-3 py-1.5 text-sm">
        <span className="text-zinc-500">新短信存入</span>
        <Segmented<string>
          value={settings?.receiveStorage ?? null}
          disabled={!online || busy}
          options={(settings?.memories.length ? settings.memories : ['SM', 'ME']).map((m) => ({
            value: m,
            label: STORAGE_LABEL[m] ?? m,
          }))}
          onChange={(m) => update({ receiveStorage: m }, `新短信将存入${STORAGE_LABEL[m] ?? m}`)}
        />
      </div>
      <div className="flex items-center justify-between gap-3 py-1.5 text-sm">
        <div>
          <div className="text-zinc-500">读取后保留在模块/SIM 卡上</div>
          <div className="text-xs text-zinc-400">关闭时，短信入库后即从模块删除，避免存储写满</div>
        </div>
        <Toggle
          label="保留在模块上"
          checked={!!settings?.keepOnModem}
          loading={busy}
          disabled={!settings}
          onChange={(v) => update({ keepOnModem: v }, v ? '短信将保留在模块上' : '短信入库后将从模块删除')}
        />
      </div>
    </Card>
  )
}
