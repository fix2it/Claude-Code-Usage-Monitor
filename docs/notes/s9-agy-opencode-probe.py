"""s9 этап 5, проба: принимает ли квотный эндпоинт Antigravity Google-токен из OpenCode.
Повторяет запросы монитора (src/poller/antigravity.rs): loadCodeAssist -> project ->
retrieveUserQuotaSummary и fetchAvailableModels, по трём адресам, с User-Agent agy.
Печатает только коды ответа и ключи. Сам токен не выводится.
Запуск: python C:\\3_Work\\VibeCode\\Usage_Monitor\\docs\\notes\\s9-agy-opencode-probe.py
"""
import json, os, sqlite3, sys, time, urllib.request, urllib.error

# --schema: печатает схему ответа fetchAvailableModels (ключи и типы, по одной модели),
# без значений-строк, чтобы не утекли токены или идентификаторы.
SCHEMA = "--schema" in sys.argv


def shape(v, depth=0):
    if isinstance(v, dict):
        return {k: shape(x, depth + 1) for k, x in v.items()} if depth < 6 else "dict"
    if isinstance(v, list):
        return [shape(v[0], depth + 1)] if v else []
    if isinstance(v, (int, float)) and not isinstance(v, bool):
        return f"{type(v).__name__}={v}" if depth >= 2 else type(v).__name__
    return type(v).__name__

db = os.path.expandvars(r"%USERPROFILE%\.local\share\opencode\opencode.db")
con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
row = con.execute(
    "SELECT value FROM credential WHERE integration_id='google' AND active=1 LIMIT 1"
).fetchone()
con.close()
if not row:
    raise SystemExit("нет активной записи google в opencode.db")
cred = json.loads(row[0])
left = (cred.get("expires", 0) / 1000 - time.time()) / 60
print(f"токен истекает через {left:.0f} мин")
if left <= 0:
    raise SystemExit("токен истёк: открой OpenCode, чтобы он обновил, и повтори")

HEAD = {
    "Authorization": "Bearer " + cred["access"],
    "Content-Type": "application/json",
    "User-Agent": "antigravity/cli/1.4.0",
}


def post(url, body):
    req = urllib.request.Request(url, json.dumps(body).encode(), HEAD, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            return r.status, json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        return e.code, e.read()[:200].decode(errors="replace")


for base in (
    "https://daily-cloudcode-pa.googleapis.com",
    "https://cloudcode-pa.googleapis.com",
    "https://daily-cloudcode-pa.sandbox.googleapis.com",
):
    print("==", base)
    code, body = post(base + "/v1internal:loadCodeAssist", {"metadata": {"ideType": "ANTIGRAVITY"}})
    project = body.get("project") if isinstance(body, dict) else None
    print(" loadCodeAssist", code, "project есть" if project else f"project нет {body if code != 200 else ''}")
    if project:
        code, body = post(base + "/v1internal:retrieveUserQuotaSummary", {"project": project})
        print(" retrieveUserQuotaSummary", code, list(body)[:8] if isinstance(body, dict) else body)
    code, body = post(base + "/v1internal:fetchAvailableModels", {"project": project} if project else {})
    print(" fetchAvailableModels", code, list(body)[:8] if isinstance(body, dict) else body)
    if SCHEMA and code == 200 and isinstance(body, dict):
        models = body.get("models")
        print("  тип models:", type(models).__name__)
        first = None
        if isinstance(models, dict) and models:
            name = next(iter(models))
            print("  ключей-моделей:", len(models), "| пример имени длиной", len(name))
            first = models[name]
        elif isinstance(models, list) and models:
            print("  моделей:", len(models))
            first = models[0]
        print("  схема одной модели:", json.dumps(shape(first), ensure_ascii=False, indent=1))
        print("  прочие ключи ответа:", {k: type(v).__name__ for k, v in body.items() if k != "models"})
        break
