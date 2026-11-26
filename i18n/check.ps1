# Verify the i18n catalog: every tr!/trf! msgid used in Rust must have a
# translation in all three locales, with matching placeholders.
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$locales = @("zh-TW", "en", "ja")

$used = @{}
$rustFiles = Get-ChildItem -Recurse -File (Join-Path $root "src") -Filter *.rs
foreach ($f in $rustFiles) {
    $rel = $f.FullName.Replace($root + "\", "")
    $line = 1
    foreach ($raw in (Get-Content $f.FullName -Raw -Encoding utf8) -split "`n") {
        foreach ($m in [regex]::Matches($raw, '\btrf?!\(\s*"((?:[^"\\]|\\.)*)"')) {
            $k = $m.Groups[1].Value
            if (-not $used.ContainsKey($k)) { $used[$k] = @() }
            $used[$k] += "$rel`:$line"
        }
        $line++
    }
}

$problems = New-Object System.Collections.Generic.List[string]
$conflicts = New-Object System.Collections.Generic.List[string]
$catalogs = @{}
$origin = @{}
foreach ($loc in $locales) {
    $catalogs[$loc] = @{}
    $origin[$loc] = @{}
    foreach ($frag in (Get-ChildItem -File (Join-Path $root "i18n\$loc") -Filter *.tsv | Sort-Object Name)) {
        $n = 0
        foreach ($line in (Get-Content $frag.FullName -Encoding utf8)) {
            $n++
            $line = $line.TrimEnd("`r")
            if ($line -eq "" -or $line.StartsWith("#")) { continue }
            $tab = $line.IndexOf("`t")
            if ($tab -lt 0) { $problems.Add("$loc/$($frag.Name):$n missing TAB"); continue }
            $k = $line.Substring(0, $tab)
            $v = $line.Substring($tab + 1)
            if ($v.Trim() -eq "") { $problems.Add("$loc/$($frag.Name):$n empty translation for [$k]") }
            if ($catalogs[$loc].ContainsKey($k) -and $catalogs[$loc][$k] -ne $v) {
                $conflicts.Add("$loc [$k]: $($origin[$loc][$k]) => $($catalogs[$loc][$k])  !=  $($frag.Name) => $v")
            }
            if (-not $origin[$loc].ContainsKey($k)) { $origin[$loc][$k] = "$($frag.Name):$n" }
            $catalogs[$loc][$k] = $v
        }
    }
}

function Slots([string]$s) { ([regex]::Matches($s, '\{\d*\}')).Count }
foreach ($loc in $locales) {
    foreach ($k in $catalogs[$loc].Keys) {
        $a = Slots $k; $b = Slots $catalogs[$loc][$k]
        if ($a -ne $b) { $problems.Add("$loc placeholder count $b != $a for [$k]") }
    }
}

$missing = New-Object System.Collections.Generic.List[string]
foreach ($k in $used.Keys) {
    foreach ($loc in $locales) {
        if (-not $catalogs[$loc].ContainsKey($k)) { $missing.Add($k); break }
    }
}

"msgids used in Rust: $($used.Count)"
foreach ($loc in $locales) { "$loc entries: $($catalogs[$loc].Count)" }
"conflicting duplicates: $($conflicts.Count)"
"missing msgids: $($missing.Count)"
"other problems: $($problems.Count)"
""
if ($missing.Count -gt 0) {
    "=== MISSING ==="
    foreach ($k in ($missing | Sort-Object)) { "  [$k]  <- $($used[$k] -join ", ")" }
}
if ($conflicts.Count -gt 0) {
    ""; "=== CONFLICTS ==="
    $conflicts | Select-Object -First 60 | ForEach-Object { "  $_" }
}
if ($problems.Count -gt 0) {
    ""; "=== OTHER ==="
    $problems | Select-Object -First 60 | ForEach-Object { "  $_" }
}
if ($missing.Count -gt 0 -or $problems.Count -gt 0) { exit 1 }
if ($conflicts.Count -gt 0) { exit 2 }