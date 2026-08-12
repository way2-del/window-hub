# WinRT OCR helper for tray-visual-smoke (Windows.Media.Ocr).
# Requires: Settings → Time & language → Language → Chinese (Simplified) → OCR.
param(
  [Parameter(Mandatory = $true)][string]$ImagePath
)

$ErrorActionPreference = "Stop"

# Bridge WinRT IAsyncOperation → .NET Task (works on Windows PowerShell 5.1).
Add-Type -AssemblyName System.Runtime.WindowsRuntime | Out-Null
$source = @"
using System;
using System.Threading.Tasks;
using Windows.Foundation;
public static class WhWinRt {
  public static T Await<T>(IAsyncOperation<T> op) {
    return op.AsTask().GetAwaiter().GetResult();
  }
}
"@

try {
  Add-Type -TypeDefinition $source -ReferencedAssemblies @(
    "System.Runtime.WindowsRuntime",
    "$env:SystemRoot\System32\WinMetadata\Windows.winmd"
  ) -ErrorAction Stop | Out-Null
} catch {
  # Fallback: load via LanguageProjection if winmd path differs.
  try {
    Add-Type -TypeDefinition $source -ErrorAction Stop | Out-Null
  } catch {
    Write-Error "Cannot compile WinRT await helper: $_"
    exit 1
  }
}

$null = [Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime]
$null = [Windows.Storage.FileAccessMode, Windows.Storage, ContentType = WindowsRuntime]
$null = [Windows.Graphics.Imaging.BitmapDecoder, Windows.Graphics.Imaging, ContentType = WindowsRuntime]
$null = [Windows.Media.Ocr.OcrEngine, Windows.Media.Ocr, ContentType = WindowsRuntime]
$null = [Windows.Globalization.Language, Windows.Globalization, ContentType = WindowsRuntime]

if (-not (Test-Path -LiteralPath $ImagePath)) {
  Write-Error "Image not found: $ImagePath"
  exit 1
}

$path = (Resolve-Path -LiteralPath $ImagePath).Path

$engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
if ($null -eq $engine) {
  $lang = [Windows.Globalization.Language]::new("zh-Hans")
  if ([Windows.Media.Ocr.OcrEngine]::IsLanguageSupported($lang)) {
    $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromLanguage($lang)
  }
}
if ($null -eq $engine) {
  Write-Error "No OCR language pack. Install 中文(简体) OCR under Windows Language settings."
  exit 1
}

$file = [WhWinRt]::Await([Windows.Storage.StorageFile]::GetFileFromPathAsync($path))
$stream = [WhWinRt]::Await($file.OpenAsync([Windows.Storage.FileAccessMode]::Read))
$decoder = [WhWinRt]::Await([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream))
$bitmap = [WhWinRt]::Await($decoder.GetSoftwareBitmapAsync())
$result = [WhWinRt]::Await($engine.RecognizeAsync($bitmap))
Write-Output $result.Text
$stream.Dispose()
