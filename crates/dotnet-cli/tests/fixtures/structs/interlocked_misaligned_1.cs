// Intentional non-differential fixture: dotnet-rs returns 1 after throwing
// DataMisalignedException for misaligned Interlocked operations, while real .NET
// on x86-64 returns 42 because the hardware permits the accesses. ECMA-335 does
// not define misaligned Interlocked behavior; this tests dotnet-rs conformance.
using System;
using System.Runtime.InteropServices;
using System.Threading;

[StructLayout(LayoutKind.Explicit)]
struct MisalignedInt
{
    [FieldOffset(0)] public byte padding;
    [FieldOffset(1)] public int value;
}

public class Program
{
    public static int Main()
    {
        MisalignedInt value = new MisalignedInt { value = 10 };
        int faults = 0;

        try
        {
            Interlocked.CompareExchange(ref value.value, 20, 10);
        }
        catch (DataMisalignedException)
        {
            faults++;
        }

        try
        {
            Interlocked.Exchange(ref value.value, 20);
        }
        catch (DataMisalignedException)
        {
            faults++;
        }

        try
        {
            Interlocked.Add(ref value.value, 1);
        }
        catch (DataMisalignedException)
        {
            faults++;
        }

        try
        {
            Volatile.Write(ref value.value, 31);
            if (Volatile.Read(ref value.value) != 31)
            {
                return 42;
            }
        }
        catch (DataMisalignedException)
        {
            return 42;
        }

        return faults == 3 ? 1 : 42;
    }
}
