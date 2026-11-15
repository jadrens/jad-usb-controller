export function formatBytes(n: number | null | undefined): string {
  if (n == null) return '—'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let v = n
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${v.toFixed(i === 0 ? 0 : v < 10 ? 2 : 1)} ${units[i]}`
}

export function formatTime(iso: string | null | undefined, withDate = true): string {
  if (!iso) return '—'
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return iso
  const now = new Date()
  const sameDay = d.toDateString() === now.toDateString()
  const time = d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
  if (sameDay || !withDate) return time
  const sameYear = d.getFullYear() === now.getFullYear()
  const date = d.toLocaleDateString('zh-CN', sameYear ? { month: 'numeric', day: 'numeric' } : undefined)
  return `${date} ${time}`
}

export function formatDuration(fromIso: string | null | undefined): string {
  if (!fromIso) return '—'
  const s = Math.max(0, Math.floor((Date.now() - Date.parse(fromIso)) / 1000))
  if (s < 60) return `${s} 秒`
  if (s < 3600) return `${Math.floor(s / 60)} 分钟`
  if (s < 86400) return `${Math.floor(s / 3600)} 小时 ${Math.floor((s % 3600) / 60)} 分`
  return `${Math.floor(s / 86400)} 天 ${Math.floor((s % 86400) / 3600)} 小时`
}

export const REGISTRATION_LABEL: Record<string, string> = {
  home: '已注册（本地）',
  roaming: '已注册（漫游）',
  home_sms_only: '仅短信（本地）',
  roaming_sms_only: '仅短信（漫游）',
  searching: '搜索网络中',
  denied: '注册被拒绝',
  not_registered: '未注册',
  emergency_only: '仅紧急呼叫',
  unknown: '未知',
}

export const SIM_STATE_LABEL: Record<string, string> = {
  READY: '就绪',
  'SIM PIN': '需要 PIN 码',
  'SIM PUK': '需要 PUK 码',
  'SIM PIN2': '需要 PIN2',
  'SIM PUK2': '需要 PUK2',
  'NOT INSERTED': '未插卡',
  BUSY: 'SIM 忙',
  FAILURE: 'SIM 故障',
  ERROR: '读取失败',
}

export const STORAGE_LABEL: Record<string, string> = {
  SM: 'SIM 卡',
  ME: '模块',
  DIRECT: '直接接收',
}
