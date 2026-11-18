using System;

public delegate void SpanAction(Span<int> span);
public static class Program
{
    static int order;
    static void First(Span<int> span) { order = order * 10 + 1; span[0] += 10; GC.Collect(); }
    static void Second(Span<int> span) { order = order * 10 + 2; span[0] += 20; }
    static void Throw(Span<int> span) { span[0]++; throw new InvalidOperationException(); }
    static bool Filter(Exception e, Span<int> span) { GC.Collect(); return e is InvalidOperationException && span[0] == 32; }
    static class Initialized
    {
        static Initialized() { GC.Collect(); order = order * 10 + 3; }
        public static void Run(Span<int> span) { span[0] += 10; }
    }
    public static int Main()
    {
        Span<int> stack = stackalloc int[1]; stack[0] = 1;
        var array = new int[] { 1 }; Span<int> heap = array;
        SpanAction action = First; action += Second;
        action(stack); action(heap);
        if (stack[0] != 31 || array[0] != 31 || order != 1212) return 1;
        SpanAction exceptional = Throw; exceptional += Second;
        try { exceptional(heap); return 2; }
        catch (Exception e) when (Filter(e, heap)) { }
        SpanAction init = First; init += Initialized.Run;
        init(heap);
        Console.WriteLine($"{stack[0]} {array[0]} {order}");
        return array[0] == 52 && order == 121213 ? 42 : 3;
    }
}
