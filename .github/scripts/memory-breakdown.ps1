<#
  Breaks the private working set of a running process down by where it lives:
  loaded modules (image pages that were written to), mapped sections (window
  frame buffers, shared sections), thread stacks and everything else private
  (the heaps and direct allocations). Diagnostic only: it never fails a run.

  Usage: ./memory-breakdown.ps1 -ProcessId 1234 -Stage idle -OutDir .
  Writes memory-breakdown-<stage>.json and prints a short table.
#>
param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][string]$Stage,
    [Parameter(Mandatory)][string]$OutDir
)
$ErrorActionPreference = "Stop"

if (-not ("MemMap" -as [type])) {
    Add-Type @"
using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

public class MemRow {
    public string Kind;
    public string Name;
    public long Private;
    public long Resident;
    public long Committed;
    public int Regions;
}

public static class MemMap {
    [StructLayout(LayoutKind.Sequential)]
    struct MBI {
        public IntPtr BaseAddress;
        public IntPtr AllocationBase;
        public uint AllocationProtect;
        public ushort PartitionId;
        public UIntPtr RegionSize;
        public uint State;
        public uint Protect;
        public uint Type;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct WsEx {
        public IntPtr VirtualAddress;
        public ulong Flags;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll")]
    static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")]
    static extern UIntPtr VirtualQueryEx(IntPtr h, IntPtr addr, out MBI mbi, UIntPtr len);
    [DllImport("psapi.dll", SetLastError = true)]
    static extern bool QueryWorkingSetEx(IntPtr h, [In, Out] WsEx[] info, int cb);
    [DllImport("psapi.dll", CharSet = CharSet.Unicode)]
    static extern uint GetMappedFileNameW(IntPtr h, IntPtr addr, StringBuilder name, uint size);

    const uint MEM_COMMIT = 0x1000;
    const uint MEM_IMAGE = 0x1000000;
    const uint MEM_MAPPED = 0x40000;
    const uint PAGE_GUARD = 0x100;
    const long PAGE = 4096;

    class Alloc {
        public long Base;
        public uint Type;
        public string Name;
        public bool Guard;
        public long Private;
        public long Resident;
        public long Committed;
        public int Regions;
    }

    public static List<MemRow> Walk(int pid) {
        IntPtr h = OpenProcess(0x0400 | 0x0010, false, pid);
        if (h == IntPtr.Zero) throw new Exception("OpenProcess failed " + Marshal.GetLastWin32Error());
        var allocs = new Dictionary<long, Alloc>();
        try {
            long addr = 0;
            const long limit = 0x7FFFFFFE0000L;
            int sz = Marshal.SizeOf(typeof(MBI));
            while (addr < limit) {
                MBI mbi;
                if (VirtualQueryEx(h, new IntPtr(addr), out mbi, new UIntPtr((uint)sz)) == UIntPtr.Zero) break;
                long size = (long)mbi.RegionSize.ToUInt64();
                long start = mbi.BaseAddress.ToInt64();
                if (size <= 0) break;
                if (mbi.State == MEM_COMMIT) {
                    long key = mbi.AllocationBase.ToInt64();
                    Alloc a;
                    if (!allocs.TryGetValue(key, out a)) {
                        a = new Alloc();
                        a.Base = key;
                        a.Type = mbi.Type;
                        a.Name = "";
                        allocs[key] = a;
                        if (mbi.Type == MEM_IMAGE || mbi.Type == MEM_MAPPED) {
                            var sb = new StringBuilder(520);
                            if (GetMappedFileNameW(h, new IntPtr(start), sb, 520) > 0) a.Name = sb.ToString();
                        }
                    }
                    if ((mbi.Protect & PAGE_GUARD) != 0) a.Guard = true;
                    a.Regions++;
                    a.Committed += size;
                    long pages = size / PAGE;
                    long done = 0;
                    while (done < pages) {
                        int batch = (int)Math.Min(pages - done, 16384);
                        var info = new WsEx[batch];
                        for (int i = 0; i < batch; i++) info[i].VirtualAddress = new IntPtr(start + (done + i) * PAGE);
                        if (QueryWorkingSetEx(h, info, batch * Marshal.SizeOf(typeof(WsEx)))) {
                            for (int i = 0; i < batch; i++) {
                                ulong f = info[i].Flags;
                                bool valid = (f & 1UL) != 0;
                                bool shared = (f & (1UL << 15)) != 0;
                                if (valid) {
                                    a.Resident += PAGE;
                                    if (!shared) a.Private += PAGE;
                                }
                            }
                        }
                        done += batch;
                    }
                }
                addr = start + size;
            }
        } finally {
            CloseHandle(h);
        }

        var rows = new Dictionary<string, MemRow>();
        var list = new List<MemRow>();
        foreach (var a in allocs.Values) {
            string kind;
            string name;
            bool group;
            if (a.Type == MEM_IMAGE) {
                kind = "image";
                name = a.Name.Length > 0 ? Path.GetFileName(a.Name) : "(unnamed image)";
                group = true;
            } else if (a.Type == MEM_MAPPED) {
                kind = "mapped";
                name = (a.Name.Length > 0 ? Path.GetFileName(a.Name) : "section") + " @" + a.Base.ToString("x");
                group = false;
            } else if (a.Guard) {
                kind = "stack";
                name = "thread stacks";
                group = true;
            } else {
                kind = "private";
                name = "private allocation @" + a.Base.ToString("x");
                group = false;
            }
            if (group) {
                string key = kind + "|" + name;
                MemRow r;
                if (!rows.TryGetValue(key, out r)) {
                    r = new MemRow();
                    r.Kind = kind;
                    r.Name = name;
                    rows[key] = r;
                    list.Add(r);
                }
                r.Private += a.Private;
                r.Resident += a.Resident;
                r.Committed += a.Committed;
                r.Regions += a.Regions;
            } else {
                var r = new MemRow();
                r.Kind = kind;
                r.Name = name;
                r.Private = a.Private;
                r.Resident = a.Resident;
                r.Committed = a.Committed;
                r.Regions = a.Regions;
                list.Add(r);
            }
        }
        return list;
    }
}
"@
}

$rows = [MemMap]::Walk($ProcessId)
$byKind = [ordered]@{}
foreach ($kind in "image", "mapped", "stack", "private") {
    $set = @($rows | Where-Object { $_.Kind -eq $kind })
    $byKind[$kind] = [ordered]@{
        private_ws = [int64](($set | Measure-Object -Property Private -Sum).Sum)
        resident   = [int64](($set | Measure-Object -Property Resident -Sum).Sum)
        committed  = [int64](($set | Measure-Object -Property Committed -Sum).Sum)
    }
}
$top = @($rows | Where-Object { $_.Private -gt 0 } | Sort-Object Private -Descending | Select-Object -First 30 | ForEach-Object {
    [ordered]@{ kind = $_.Kind; name = $_.Name; private_ws = $_.Private; resident = $_.Resident; committed = $_.Committed; regions = $_.Regions }
})
$out = [ordered]@{
    stage   = $Stage
    total_private_ws = [int64](($rows | Measure-Object -Property Private -Sum).Sum)
    by_kind = $byKind
    top     = $top
}
$path = Join-Path $OutDir "memory-breakdown-$Stage.json"
$out | ConvertTo-Json -Depth 6 | Set-Content $path -Encoding utf8
Write-Host ("breakdown {0}: private ws {1:N2} MB" -f $Stage, ($out.total_private_ws / 1MB))
foreach ($k in $byKind.Keys) {
    Write-Host ("  {0,-8} private {1,7:N2} MB  resident {2,7:N2} MB  committed {3,7:N2} MB" -f $k, ($byKind[$k].private_ws / 1MB), ($byKind[$k].resident / 1MB), ($byKind[$k].committed / 1MB))
}
foreach ($t in ($top | Select-Object -First 12)) {
    Write-Host ("    {0,7:N2} MB  {1}  {2}" -f ($t.private_ws / 1MB), $t.kind, $t.name)
}
