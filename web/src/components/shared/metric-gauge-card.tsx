import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { cssVarToRgb } from '@/lib/utils'

interface MetricGaugeCardProps {
  label: string
  /** 0–100 사용률(%). `display`가 없을 때 값·게이지·심각도에 쓰인다. */
  value?: number
  /** 값 표시 오버라이드(예: "412.3 GB") — 지정하면 % 대신 텍스트만 렌더링하고 게이지는 숨긴다. */
  display?: string
  className?: string
}

/**
 * MetricGaugeCard — 단일 자원 사용률(0–100%)을 라벨 + 큰 값 + 심각도 게이지 바로 표시.
 *
 * Dashboard StatCard(시계열 KPI + 스파크라인)와 구분되는 "현재 순간 게이지" 패턴의
 * 공유 카드. Resources(CPU/Memory/Disk)의 3-copy 중복을 제거하고 카드 문법을 정규화하기
 * 위해 도입. 심각도 색은 info/warning/error 시맨틱 토큰에서 해석(임계값 75/90).
 * `display`를 주면 백분율이 아닌 지표(예: 디스크 사용량 GB)를 같은 카드 문법으로 렌더링한다.
 */
export function MetricGaugeCard({ label, value, display, className }: MetricGaugeCardProps) {
  const numeric = display === undefined && Number.isFinite(value) ? value! : undefined
  const text = display ?? (numeric !== undefined ? `${numeric.toFixed(1)}%` : '—')
  // 0–100 사용률 심각도: info(<75) · warning(75–90) · error(90+).
  const sevToken =
    numeric === undefined
      ? '--color-status-info'
      : numeric >= 90
        ? '--color-status-error'
        : numeric >= 75
          ? '--color-status-warning'
          : '--color-status-info'
  const sevColor = cssVarToRgb(sevToken)

  return (
    <Card className={className}>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm text-muted-foreground">{label}</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="text-2xl font-bold">{text}</div>
        {numeric !== undefined && (
          <div className="mt-2 h-2 rounded-full bg-muted overflow-hidden">
            <div
              className="h-full rounded-full transition-all"
              style={{ width: `${numeric}%`, backgroundColor: sevColor }}
            />
          </div>
        )}
      </CardContent>
    </Card>
  )
}
