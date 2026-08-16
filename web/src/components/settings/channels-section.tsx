import { Send } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Separator } from '@/components/ui/separator'
import type { SettingsFieldDef } from './field-defs'
import { FieldRow } from './field-row'
import { TelegramConnectCard } from './telegram-connect-card'

interface ChannelsSectionProps {
  /** Section key, e.g. `channels.telegram`. */
  sectionKey: string
  labelKey: string
  fields: SettingsFieldDef[]
  formValues: Record<string, Record<string, string | boolean | string[]>>
  onFieldChange: (sectionKey: string, fieldKey: string, value: string | boolean | string[]) => void
}

/**
 * Renders a single channel section (currently just Telegram). Uses the
 * standard `FieldRow` for every field so the restart badges and form
 * controls stay consistent. The Telegram section additionally renders the
 * instant-connect card (`TelegramConnectCard`) above the fields.
 */
export function ChannelsSection({
  sectionKey,
  labelKey,
  fields,
  formValues,
  onFieldChange,
}: ChannelsSectionProps) {
  const { t } = useTranslation()
  if (fields.length === 0) return null

  return (
    <Card>
      <CardHeader className="pb-4">
        <CardTitle className="flex items-center gap-2 text-base">
          <Send className="h-4 w-4" />
          {t(labelKey)}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        {sectionKey === 'channels.telegram' && <TelegramConnectCard />}
        {fields.map((field, i) => (
          <div key={field.key}>
            {i > 0 && <Separator className="mb-4" />}
            <FieldRow
              sectionKey={sectionKey}
              field={field}
              value={
                formValues[sectionKey]?.[field.key] as
                  | string
                  | boolean
                  | string[]
                  | number
                  | undefined
              }
              onChange={(val) => onFieldChange(sectionKey, field.key, val)}
              sectionValues={formValues[sectionKey]}
            />
          </div>
        ))}
      </CardContent>
    </Card>
  )
}
