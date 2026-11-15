// Mirrors the server-side segmentation in server/src/pdu.rs.
const GSM7_BASIC =
  '@£$¥èéùìòÇ\nØø\rÅåΔ_ΦΓΛΩΠΨΣΘΞ\u001bÆæßÉ !"#¤%&\'()*+,-./0123456789:;<=>?' +
  '¡ABCDEFGHIJKLMNOPQRSTUVWXYZÄÖÑÜ§¿abcdefghijklmnopqrstuvwxyzäöñüà'
const GSM7_EXT = '\f^{}\\[~]|€'

export interface SegmentInfo {
  encoding: 'GSM-7' | 'UCS-2'
  units: number
  segments: number
  perSegment: number
  remaining: number
}

export function segmentInfo(text: string): SegmentInfo {
  let septets = 0
  let gsm = true
  for (const ch of text) {
    if (ch !== '\u001b' && GSM7_BASIC.includes(ch)) septets += 1
    else if (GSM7_EXT.includes(ch)) septets += 2
    else {
      gsm = false
      break
    }
  }
  if (gsm) {
    const per = septets <= 160 ? 160 : 153
    const segments = Math.max(1, Math.ceil(septets / per))
    return { encoding: 'GSM-7', units: septets, segments, perSegment: per, remaining: segments * per - septets }
  }
  const units = text.length // UTF-16 code units
  const per = units <= 70 ? 70 : 67
  const segments = Math.max(1, Math.ceil(units / per))
  return { encoding: 'UCS-2', units, segments, perSegment: per, remaining: segments * per - units }
}
