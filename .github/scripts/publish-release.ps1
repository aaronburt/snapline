$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$package = "Snapline_$($env:PACKAGE_VERSION)_x64"
$release = "Snapline_$($env:RELEASE_VERSION)_x64"
$assetSources = @{
    "$release.exe" = 'target/release/snapline.exe'
    "$release-setup.exe" = "target/release/bundle/nsis/$package-setup.exe"
    "$release-portable.zip" = "target/portable/$package-portable.zip"
}
$assets = foreach ($name in $assetSources.Keys) {
    $source = Get-Item -LiteralPath $assetSources[$name]
    if ($source.Length -le 0) {
        throw "Empty release asset $source."
    }
    $destination = Join-Path $env:RUNNER_TEMP $name
    Copy-Item -LiteralPath $source.FullName -Destination $destination
    $destination
}

if ($env:RESUME_DRAFT -ne 'true') {
    gh release create $env:RELEASE_TAG --target $env:GITHUB_SHA --title "Snapline $env:RELEASE_VERSION" --generate-notes --draft
}
gh release upload $env:RELEASE_TAG @assets --clobber
gh release edit $env:RELEASE_TAG --draft=false "--latest=$env:MARK_LATEST"
