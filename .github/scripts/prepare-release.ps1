$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$pullRequests = gh api "repos/$env:GITHUB_REPOSITORY/commits/$env:GITHUB_SHA/pulls" --paginate --jq '.[] | {merged_at, base: .base.ref, merge_commit_sha}' |
    ForEach-Object { ConvertFrom-Json -InputObject $_ }
$mergedPullRequest = $pullRequests | Where-Object {
    $_.merged_at -and $_.base -eq 'main' -and $_.merge_commit_sha -eq $env:GITHUB_SHA
}
if (!$mergedPullRequest) {
    Write-Host 'The checked commit is not a PR merge into main.'
    exit 0
}

$major = '0'
$tagPattern = "^v$major\.(0|[1-9][0-9]*)$"
$tags = @(git tag --list "v$major.*" | Where-Object { $_ -cmatch $tagPattern })
$commitTags = @(git tag --points-at $env:GITHUB_SHA --list "v$major.*" |
    Where-Object { $_ -cmatch $tagPattern })
$releases = @(gh api "repos/$env:GITHUB_REPOSITORY/releases" --paginate --jq '.[] | {tag_name, draft, target_commitish}' |
    ForEach-Object { ConvertFrom-Json -InputObject $_ } |
    Where-Object { $_.tag_name -cmatch $tagPattern })

$existing = $releases | Where-Object { !$_.draft -and $_.tag_name -cin $commitTags }
if ($existing) {
    Write-Host "Release $($existing.tag_name) already exists for this commit."
    exit 0
}

$draft = $releases | Where-Object {
    $_.draft -and ($_.tag_name -cin $commitTags -or $_.target_commitish -eq $env:GITHUB_SHA)
}
if (@($draft).Count -gt 1 -or $commitTags.Count -gt 1) {
    throw 'Multiple release versions are associated with this commit.'
}
if ($draft -and $draft.tag_name -cin $tags -and $draft.tag_name -cnotin $commitTags) {
    throw "Draft tag $($draft.tag_name) points to a different commit."
}

$tag = if ($draft) { $draft.tag_name } elseif ($commitTags.Count) { $commitTags[0] }
if (!$tag) {
    $highestMinor = [bigint]0
    foreach ($usedTag in ($tags + @($releases | ForEach-Object { $_.tag_name }))) {
        $minor = [bigint]::Parse($usedTag.Split('.')[1])
        if ($minor -gt $highestMinor) {
            $highestMinor = $minor
        }
    }
    $tag = "v$major.$($highestMinor + 1)"
}

$version = $tag.Substring(1)
$releaseMinor = [bigint]::Parse($version.Split('.')[1])
$packageVersion = if ($releaseMinor -le 65535) { "$version.0" } else { "$major.65535.65535" }
$newerRelease = $releases | Where-Object {
    !$_.draft -and [bigint]::Parse($_.tag_name.Split('.')[1]) -gt $releaseMinor
}
$markLatest = if ($newerRelease) { 'false' } else { 'true' }

$tauriConfig = 'apps/snapline/tauri.conf.json'
$config = Get-Content -LiteralPath $tauriConfig -Raw | ConvertFrom-Json
$config.version = $packageVersion
$config | ConvertTo-Json -Depth 100 | Set-Content -LiteralPath $tauriConfig -Encoding utf8

$appManifest = 'apps/snapline/Cargo.toml'
$manifest = Get-Content -LiteralPath $appManifest -Raw
if ($manifest -notmatch '(?m)^version\.workspace = true\r?$') {
    throw "Could not set the Snapline package version in $appManifest."
}
$manifest -replace '(?m)^version\.workspace = true(?=\r?$)', "version = `"$packageVersion`"" |
    Set-Content -LiteralPath $appManifest -Encoding utf8 -NoNewline

@(
    "tag=$tag"
    "version=$version"
    "package_version=$packageVersion"
    "resume_draft=$(([bool]$draft).ToString().ToLowerInvariant())"
    "mark_latest=$markLatest"
) | Add-Content -LiteralPath $env:GITHUB_OUTPUT -Encoding utf8
