# s9, этап 0: опыт со вторым аккаунтом agy (2026-11-13)

Цель — узнать, где agy хранит токен второго аккаунта, **не читая сам токен**.
Смотрим только имена целей Credential Manager и имена файлов.

## Что известно до опыта

- Первый аккаунт: цель Credential Manager `gemini:antigravity`
  (`src/poller/antigravity.rs` ~799); в `~/.gemini` есть файл
  `jetski-standalone-oauth-token` и `google_accounts.json` (содержимое не читали).
- В `agy.exe` есть переменные окружения:
  - `ANTIGRAVITY_GEMINI_DIR` — по всей видимости, заменяет папку `~/.gemini`;
  - `AGY_ACCOUNT` — выбор одного из нескольких вошедших аккаунтов
    (сообщение «AGY_ACCOUNT=%q matches no signed-in account; available accounts: %s»).
  Значит, agy может уметь держать два аккаунта и без отдельной папки. Опыт
  проверяет оба пути.

## Шаги (PowerShell, всё делает владелец)

1. Снимок до (только имена, без паролей):

   ```powershell
   cmdkey /list | Select-String "Target" | Set-Content -Encoding utf8 "$env:TEMP\cred-before.txt"
   Get-ChildItem "$env:USERPROFILE\.gemini" -Recurse -File | Select-Object FullName, Length, LastWriteTime | Set-Content -Encoding utf8 "$env:TEMP\gemini-before.txt"
   ```

2. Вход во второй аккаунт через отдельную папку:

   ```powershell
   New-Item -ItemType Directory -Force "$env:USERPROFILE\.gemini_x"
   $env:ANTIGRAVITY_GEMINI_DIR = "$env:USERPROFILE\.gemini_x"
   agy
   ```

   В agy войти вторым аккаунтом Google, задать один короткий вопрос, выйти.
   Закрыть это окно PowerShell (переменная живёт только в нём).

3. Снимок после, в новом окне PowerShell:

   ```powershell
   cmdkey /list | Select-String "Target" | Set-Content -Encoding utf8 "$env:TEMP\cred-after.txt"
   Compare-Object (Get-Content "$env:TEMP\cred-before.txt") (Get-Content "$env:TEMP\cred-after.txt")
   Get-ChildItem "$env:USERPROFILE\.gemini_x" -Recurse -File | Select-Object FullName, Length, LastWriteTime
   Get-ChildItem "$env:USERPROFILE\.gemini" -Recurse -File | Where-Object LastWriteTime -gt (Get-Date).AddMinutes(-30) | Select-Object FullName, LastWriteTime
   ```

4. Проверка, что первый аккаунт не сломан: обычный `agy` без переменной, один
   вопрос — должен ответить первый аккаунт.

5. Прислать вывод шагов 3 и 4 (там только имена целей и файлов).

## Как читать результат

| Что появилось | Вывод для этапа 5 |
| --- | --- |
| Новая цель в Credential Manager (например `gemini:antigravity:<что-то>`) | Источник второго аккаунта — эта цель. |
| Цель не появилась, но в `~/.gemini_x` есть файл токена | Источник — файл в `~/.gemini_x`. |
| Цель `gemini:antigravity` перезаписана (первый аккаунт в шаге 4 стал вторым) | Папка не разделяет учётные данные; пробуем путь `AGY_ACCOUNT`. |
| Ничего нового, вход не прошёл | Порог не пройден: этап 5 откладывается, этап 6 только для Claude. |

Если цель перезаписана: снова войти первым аккаунтом обычным `agy`, затем
попробовать второй вход внутри agy (команда смены/добавления аккаунта, если
есть) и повторить шаги 1 и 3 без папки `~/.gemini_x`.
