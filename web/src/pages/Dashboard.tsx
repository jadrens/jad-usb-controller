import {
  Activity,
  ArrowDown,
  ArrowUp,
  Cpu,
  CreditCard,
  Globe,
  KeyRound,
  Plane,
  Power,
  RefreshCw,
  RotateCcw,
  SignalHigh,
} from 'lucide-react'
import { useState } from 'react'
import { api, type RadioMode } from '../api'
import { Badge, Button, Card, Field, Segmented, SignalBars, Sparkline, Toggle } from '../components/ui'
import { inputClass } from '../lib/style'
import { formatBytes, formatDuration, formatTime, REGISTRATION_LABEL, SIM_STATE_LABEL, STORAGE_LABEL } from '../lib/format'
import { useAction, useLive } from '../live-context'

export default function Dashboard() {
  const { status, connection, signalHistory, setStatus } = useLive()
  const run = useAction()
  const [busy, setBusy] = useState<string | null>(null)
  const online = !!connection?.connected

  const act = async <T,>(key: string, fn: () => Promise<T>, ok?: string) => {
    setBusy(key)
    const r = await run(fn, ok)
    setBusy(null)
    return r
  }

  if (!status) {
    return <div className="py-20 text-center text-sm text-zinc-400">加载中…</div>
  }

  const { sim, network, signal, data, device, radio } = status
  const registered = network.registration?.startsWith('home') || network.registration?.startsWith('roaming')
  const simTone = sim.ready ? 'ok' : sim.state === 'NOT INSERTED' ? 'bad' : 'warn'
  const needsPin = sim.state === 'SIM PIN'
  // with a USB network card (ECM/RNDIS) the host only gets data once the module dials it up
  const dataOn = data.netdev ?? data.active

  return (
    <div className="space-y-4">
      {needsPin && <PinCard />}

      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
        <Card title="信号" icon={<SignalHigh className="size-4" />}>
          <div className="flex items-end justify-between">
            <div>
              <div className="text-3xl font-semibold tabular-nums">
                {signal.rsrp ?? signal.rssi ?? '—'}
                <span className="ml-1 text-sm font-normal text-zinc-500">dBm</span>
              </div>
              <div className="text-xs text-zinc-500">{signal.rsrp != null ? 'RSRP' : 'RSSI'}</div>
            </div>
            <SignalBars level={signal.level} size="lg" />
          </div>
          <div className="mt-3 grid grid-cols-3 gap-2 text-center text-xs">
            <Metric label="RSRQ" value={signal.rsrq} unit="dB" />
            <Metric label="SINR" value={signal.sinr} unit="dB" />
            <Metric label="CSQ" value={signal.csq} />
          </div>
        </Card>

        <Card title="网络" icon={<Globe className="size-4" />}>
          <div className="text-lg font-semibold">{network.operator ?? sim.carrier ?? '无服务'}</div>
          <div className="mt-1 flex flex-wrap gap-1.5">
            <Badge tone={registered ? 'ok' : 'warn'}>
              {REGISTRATION_LABEL[network.registration ?? 'unknown'] ?? network.registration}
            </Badge>
            {network.accessTech && <Badge tone="info">{network.accessTech}</Badge>}
          </div>
          <div className="mt-2 text-xs text-zinc-500">
            {network.band ?? '—'}
            {network.channel != null && ` · 频点 ${network.channel}`}
          </div>
        </Card>

        <Card
          title="移动数据"
          icon={<Activity className="size-4" />}
          action={
            <Toggle
              label="移动数据"
              checked={!!dataOn}
              loading={busy === 'data'}
              disabled={!online || !sim.ready || radio.mode !== 'on'}
              onChange={(v) =>
                act('data', () => api.setData(v).then(setStatus), v ? '移动数据已开启' : '移动数据已关闭')
              }
            />
          }
        >
          <div className="text-lg font-semibold">
            {dataOn ? '已连接' : '已断开'}
            {!dataOn && data.wanted && <span className="ml-2 text-xs font-normal text-amber-600">自动重连中</span>}
          </div>
          {data.netdev != null && (
            <div className="mt-1 flex flex-wrap gap-1.5">
              <Badge tone={data.active ? 'ok' : 'neutral'}>PDP {data.active ? '已激活' : '未激活'}</Badge>
              <Badge tone={data.netdev ? 'ok' : 'neutral'}>网卡{data.netdev ? '已拨号' : '未拨号'}</Badge>
            </div>
          )}
          <div className="mt-1 truncate font-mono text-xs text-zinc-500">{data.ip.length ? data.ip.join(' / ') : '无 IP 地址'}</div>
          <div className="mt-2 text-xs text-zinc-500">
            APN {data.apn || '—'} · {data.pdpType ?? '—'} · {data.attached ? '已附着' : '未附着'}
          </div>
        </Card>

        <Card
          title="流量统计"
          icon={<ArrowDown className="size-4" />}
          action={
            <Button
              variant="ghost"
              className="px-1.5! py-0.5! text-xs"
              loading={busy === 'counters'}
              disabled={!online || data.rxBytes == null}
              onClick={() => confirm('确定清零模块的流量计数器？') && act('counters', () => api.resetCounters().then(setStatus), '已清零')}
            >
              清零
            </Button>
          }
        >
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <span className="flex items-center gap-1 text-sm text-zinc-500">
                <ArrowDown className="size-3.5 text-sky-500" /> 下行
              </span>
              <span className="text-lg font-semibold tabular-nums">{formatBytes(data.rxBytes)}</span>
            </div>
            <div className="flex items-center justify-between">
              <span className="flex items-center gap-1 text-sm text-zinc-500">
                <ArrowUp className="size-3.5 text-emerald-500" /> 上行
              </span>
              <span className="text-lg font-semibold tabular-nums">{formatBytes(data.txBytes)}</span>
            </div>
            <div className="text-xs text-zinc-400">
              合计 {formatBytes(data.rxBytes != null && data.txBytes != null ? data.rxBytes + data.txBytes : null)}
            </div>
          </div>
        </Card>
      </div>

      <Card
        title="信号趋势"
        icon={<SignalHigh className="size-4" />}
        action={<span className="text-xs text-zinc-400">最近 {signalHistory.length} 次采样 · dBm</span>}
      >
        <Sparkline
          className="h-20 w-full"
          values={signalHistory.map((s) => s.rsrp ?? s.rssi)}
          min={signalHistory.some((s) => s.rsrp != null) ? -130 : -113}
          max={signalHistory.some((s) => s.rsrp != null) ? -70 : -51}
        />
      </Card>

      <div className="grid gap-4 lg:grid-cols-3">
        <Card
          title="SIM 卡"
          icon={<CreditCard className="size-4" />}
          action={<Badge tone={simTone}>{SIM_STATE_LABEL[sim.state ?? ''] ?? sim.state ?? '未知'}</Badge>}
        >
          <Field label="运营商" value={sim.carrier} />
          <Field label="ICCID" value={sim.iccid} mono copy={sim.iccid} />
          <Field label="IMSI" value={sim.imsi} mono copy={sim.imsi} />
          <Field label="号码" value={sim.msisdn ?? <span className="text-zinc-400">未写入卡</span>} mono copy={sim.msisdn} />
          {(status.smsStorages.length ? status.smsStorages : status.smsStorage ? [status.smsStorage] : []).map((u) => (
            <Field
              key={u.memory}
              label={`短信 · ${STORAGE_LABEL[u.memory] ?? u.memory}`}
              value={
                <>
                  {u.used}/{u.total}
                  {status.smsStorage?.memory === u.memory && <span className="ml-1 text-xs text-sky-500">接收</span>}
                </>
              }
            />
          ))}
        </Card>

        <Card title="网络详情" icon={<Globe className="size-4" />}>
          <Field label="PLMN" value={network.plmn} mono />
          <Field label="CS 注册" value={REGISTRATION_LABEL[network.registration ?? ''] ?? network.registration} />
          <Field label="PS/EPS 注册" value={REGISTRATION_LABEL[network.psRegistration ?? ''] ?? network.psRegistration} />
          <Field label="LAC/TAC" value={network.areaCode} mono />
          <Field label="小区 ID" value={network.cellId} mono />
        </Card>

        <Card title="模块" icon={<Cpu className="size-4" />}>
          <Field label="厂商 / 型号" value={[device.manufacturer, device.model].filter(Boolean).join(' ') || null} />
          <Field label="固件" value={device.revision} mono />
          <Field label="IMEI" value={device.imei} mono copy={device.imei} />
          <Field label="USB 网卡模式" value={device.usbNetMode} />
          <Field label="连接方式" value={connection?.transport} />
          <Field label="在线时长" value={connection?.connected ? formatDuration(connection.since) : '—'} />
        </Card>
      </div>

      <Card title="射频与模块控制" icon={<Power className="size-4" />}>
        <div className="flex flex-wrap items-center gap-3">
          <Segmented<RadioMode>
            value={radio.mode === 'unknown' ? null : radio.mode}
            disabled={!online || busy === 'radio'}
            options={[
              { value: 'on', label: '正常' },
              { value: 'airplane', label: '飞行模式' },
              { value: 'off', label: '最小功能' },
            ]}
            onChange={(m) => act('radio', () => api.setRadio(m).then(setStatus), '射频模式已切换')}
          />
          {radio.mode === 'airplane' && (
            <Badge tone="warn">
              <Plane className="size-3" /> 飞行模式中
            </Badge>
          )}
          <div className="ml-auto flex gap-2">
            <Button
              icon={<RefreshCw className="size-4" />}
              loading={busy === 'refresh'}
              disabled={!online}
              onClick={() => act('refresh', () => api.refresh().then(setStatus))}
            >
              刷新
            </Button>
            <Button
              variant="danger"
              icon={<RotateCcw className="size-4" />}
              loading={busy === 'reboot'}
              disabled={!online}
              onClick={() => confirm('重启模块？连接将中断约 20 秒。') && act('reboot', api.reboot, '模块正在重启')}
            >
              重启模块
            </Button>
          </div>
        </div>
        <p className="mt-2 text-xs text-zinc-400">更新于 {formatTime(status.updatedAt)}</p>
      </Card>
    </div>
  )
}

function Metric({ label, value, unit }: { label: string; value: number | null; unit?: string }) {
  return (
    <div className="rounded-lg bg-zinc-50 py-1.5 dark:bg-zinc-800/60">
      <div className="text-zinc-500">{label}</div>
      <div className="font-semibold tabular-nums">
        {value ?? '—'}
        {value != null && unit && <span className="ml-0.5 font-normal text-zinc-400">{unit}</span>}
      </div>
    </div>
  )
}

function PinCard() {
  const { setStatus } = useLive()
  const run = useAction()
  const [pin, setPin] = useState('')
  const [busy, setBusy] = useState(false)
  return (
    <Card title="SIM 卡已锁定" icon={<KeyRound className="size-4 text-amber-500" />} className="border-amber-300 dark:border-amber-800">
      <form
        className="flex flex-wrap items-center gap-2"
        onSubmit={async (e) => {
          e.preventDefault()
          setBusy(true)
          const s = await run(() => api.enterPin(pin), 'PIN 已验证')
          setBusy(false)
          if (s) setStatus(s)
        }}
      >
        <input
          className={inputClass + ' max-w-40 font-mono'}
          inputMode="numeric"
          type="password"
          placeholder="PIN 码"
          value={pin}
          onChange={(e) => setPin(e.target.value.replace(/\D/g, '').slice(0, 8))}
        />
        <Button variant="primary" loading={busy} disabled={pin.length < 4}>
          解锁
        </Button>
        <span className="text-xs text-amber-700 dark:text-amber-300">注意：连续输错 3 次将锁卡，需要 PUK 码</span>
      </form>
    </Card>
  )
}
