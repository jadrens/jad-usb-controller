export interface ConnectionState {
  connected: boolean
  transport: string | null
  error: string | null
  since: string | null
}

export interface Status {
  updatedAt: string | null
  device: {
    manufacturer: string | null
    model: string | null
    revision: string | null
    imei: string | null
    usbNetMode: string | null
  }
  sim: {
    state: string | null
    ready: boolean
    iccid: string | null
    imsi: string | null
    msisdn: string | null
    carrier: string | null
  }
  network: {
    operator: string | null
    plmn: string | null
    accessTech: string | null
    registration: string | null
    psRegistration: string | null
    areaCode: string | null
    cellId: string | null
    band: string | null
    channel: number | null
  }
  signal: {
    csq: number | null
    rssi: number | null
    ber: number | null
    rsrp: number | null
    rsrq: number | null
    sinr: number | null
    level: number
  }
  data: {
    attached: boolean | null
    active: boolean | null
    /** Quectel ECM/RNDIS: PDP routed to the host network card (AT+QNETDEVCTL) */
    netdev: boolean | null
    cid: number
    /** Switched on by the user; the server re-enables it when it drops */
    wanted: boolean
    ip: string[]
    apn: string | null
    pdpType: string | null
    txBytes: number | null
    rxBytes: number | null
  }
  radio: { cfun: number | null; mode: RadioMode | 'unknown' }
  smsStorage: SmsStorage | null
  smsStorages: SmsStorage[]
}

/** `SM` = SIM card, `ME` = module memory */
export interface SmsStorage {
  memory: string
  used: number
  total: number
}

export interface SmsSettings {
  keepOnModem: boolean
  receiveStorage: string | null
  memories: string[]
  usage: SmsStorage[]
}

export interface Carrier {
  earfcn: number
  band: number | null
  duplex: 'FDD' | 'TDD' | 'SDL' | null
  dlMhz: number | null
  ulMhz: number | null
}

export interface ServingCell {
  /** SEARCH | LIMSRV | NOCONN | CONNECT */
  state: string
  rat: string
  duplex: string | null
  mcc: string | null
  mnc: string | null
  cellId: string | null
  enbId: number | null
  sector: number | null
  pci: number | null
  tac: string | null
  carrier: Carrier | null
  ulBandwidthMhz: number | null
  dlBandwidthMhz: number | null
  rsrp: number | null
  rsrq: number | null
  rssi: number | null
  sinr: number | null
  srxlev: number | null
  raw: string[]
}

export interface NeighbourCell {
  kind: 'intra' | 'inter'
  carrier: Carrier
  pci: number
  rsrp: number | null
  rsrq: number | null
  sinr: number | null
  srxlev: number | null
}

export interface CellInfo {
  serving: ServingCell | null
  neighbours: NeighbourCell[]
  configured: Carrier[]
  otherRat: string[]
  lteBands: number[]
  scanMode: string | null
}

export interface EuiccProfile {
  iccid: string
  isdpAid: string | null
  enabled: boolean
  nickname: string | null
  provider: string | null
  name: string | null
  class: 'test' | 'provisioning' | 'operational' | null
  /** data: URL */
  icon: string | null
}

export interface EuiccInfo {
  eid: string | null
  defaultSmdp: string | null
  rootSmds: string | null
  firmware: string | null
  svn: string | null
  freeNvm: number | null
  profiles: EuiccProfile[]
}

/** One eUICC on the card, reached through its ISD-R AID */
export interface EuiccCard {
  label: string
  aid: string
  /** other ISD-R AIDs that reach the same eUICC */
  aliases: string[]
  /** holds the profile the modem is currently using */
  active: boolean
  info: EuiccInfo
}

export interface EuiccState {
  supported: boolean
  cards: EuiccCard[]
}

export type RadioMode = 'on' | 'airplane' | 'off'

export interface Sms {
  id: number
  direction: 'in' | 'out'
  peer: string
  text: string
  ts: string
  status: 'unread' | 'read' | 'sending' | 'sent' | 'failed'
  parts: number
  error: string | null
  /** Where the message was read from: `SM` SIM card, `ME` module, `DIRECT` delivered without storing */
  storage: string | null
  /** Still stored on the modem (only when "keep on modem" is enabled) */
  onModem: boolean
}

export interface Conversation {
  peer: string
  lastText: string
  lastTs: string
  lastDirection: 'in' | 'out'
  unread: number
  total: number
}

export interface ServerInfo {
  connection: ConnectionState
  status: Status
  unread: number
  config: { transport: string; keepSmsOnSim: boolean; dataCid: number; pollSecs: number }
}

export interface PdpContext {
  cid: number
  pdpType: string
  apn: string
  address: string | null
}

export interface UssdReply {
  status: number | null
  text: string
  dcs: number | null
}

export interface RawReply {
  ok: boolean
  lines: string[]
  error: string | null
}

export interface DeviceListing {
  usb: {
    vid: number
    pid: number
    bus: string
    address: number
    manufacturer: string | null
    product: string | null
    knownVendor: string | null
    interfaces: { number: number; class: number; name: string | null }[]
  }[]
  serial: {
    path: string
    vid: number | null
    pid: number | null
    product: string | null
    knownVendor: string | null
  }[]
}

export interface AtLogEntry {
  dir: 'tx' | 'rx' | 'urc'
  line: string
  ts: string
}

export type ServerEvent =
  | { type: 'status'; status: Status }
  | { type: 'connection'; connection: ConnectionState }
  | { type: 'sms'; sms: Sms }
  | { type: 'sms_sync'; new: number }
  | ({ type: 'at_log' } & AtLogEntry)

export class ApiError extends Error {
  readonly status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`/api${path}`, {
    method,
    headers: body === undefined ? undefined : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  const data = await res.json().catch(() => null)
  if (!res.ok) {
    throw new ApiError(res.status, data?.error ?? `${res.status} ${res.statusText}`)
  }
  return data as T
}

const enc = encodeURIComponent

export const api = {
  info: () => request<ServerInfo>('GET', '/status'),
  refresh: () => request<Status>('POST', '/status/refresh'),
  devices: () => request<DeviceListing>('GET', '/devices'),

  conversations: () => request<Conversation[]>('GET', '/sms/conversations'),
  messages: (peer: string, before?: number) =>
    request<Sms[]>('GET', `/sms?peer=${enc(peer)}&limit=200${before ? `&before=${before}` : ''}`),
  send: (to: string, text: string) => request<Sms>('POST', '/sms', { to, text }),
  syncSms: () => request<{ new: number }>('POST', '/sms/sync'),
  smsSettings: () => request<SmsSettings>('GET', '/sms/settings'),
  setSmsSettings: (body: { keepOnModem?: boolean; receiveStorage?: string }) =>
    request<SmsSettings>('PUT', '/sms/settings', body),
  deleteSms: (id: number) => request<{ deleted: boolean }>('DELETE', `/sms/${id}`),
  deletePeer: (peer: string) => request<{ deleted: number }>('DELETE', `/sms/peer/${enc(peer)}`),
  markRead: (peer: string) => request<{ updated: number }>('POST', `/sms/peer/${enc(peer)}/read`),

  setData: (enabled: boolean) => request<Status>('POST', '/data', { enabled }),
  resetCounters: () => request<Status>('POST', '/data/counters/reset'),
  setRadio: (mode: RadioMode) => request<Status>('POST', '/radio', { mode }),
  reboot: () => request<{ ok: boolean }>('POST', '/modem/reboot'),
  enterPin: (pin: string) => request<Status>('POST', '/sim/pin', { pin }),
  setMsisdn: (number: string) => request<Status>('POST', '/sim/msisdn', { number }),

  cell: () => request<CellInfo>('GET', '/cell'),
  euicc: () => request<EuiccState>('GET', '/euicc'),
  euiccProfile: (aid: string, iccid: string, action: 'enable' | 'disable' | 'delete') =>
    request<EuiccState>('POST', `/euicc/${aid}/profiles/${enc(iccid)}/${action}`),
  euiccNickname: (aid: string, iccid: string, nickname: string) =>
    request<EuiccState>('PUT', `/euicc/${aid}/profiles/${enc(iccid)}/nickname`, { nickname }),

  apn: () => request<PdpContext[]>('GET', '/apn'),
  setApn: (body: {
    cid: number
    pdpType: string
    apn: string
    username?: string
    password?: string
    auth?: number
  }) => request<PdpContext[]>('PUT', '/apn', body),

  ussd: (code: string) => request<UssdReply>('POST', '/ussd', { code }),
  ussdCancel: () => request<{ ok: boolean }>('POST', '/ussd/cancel'),
  at: (command: string, timeoutMs?: number) => request<RawReply>('POST', '/at', { command, timeoutMs }),
}
