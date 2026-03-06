using System;

public class Program
{
    public static int Main()
    {
        byte[] data = new byte[3];
        data[0] = 10;
        data[1] = 20;
        data[2] = 30;
        ReadOnlySpan<byte> roSpan = data;
        ref readonly byte roRef = ref roSpan.GetPinnableReference();
        if (roRef != 10) return 1;
        data[0] = 99;
        if (roRef != 99) return 2;

        Span<byte> span = data;
        ref byte sRef = ref span.GetPinnableReference();
        if (sRef != 99) return 3;
        span[0] = 7;
        if (sRef != 7) return 4;

        Span<byte> stackSpan = stackalloc byte[2];
        stackSpan[0] = 1;
        stackSpan[1] = 2;
        ref byte stackRef = ref stackSpan.GetPinnableReference();
        if (stackRef != 1) return 5;
        stackSpan[0] = 4;
        if (stackRef != 4) return 6;

        return 0;
    }
}