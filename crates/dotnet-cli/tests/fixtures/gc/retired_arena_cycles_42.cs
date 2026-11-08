using System;
using System.Threading;

class Node
{
    public int Value;
    public Node? Next;
}

class Program
{
    static Node? Root;
    static Node? First;
    static Node? Second;
    static Node? Live;
    static WeakReference? Control;
    static volatile bool Stop;

    static void MakeFirst() { First = new Node { Value = 2 }; }
    static void MakeSecond() { Second = new Node { Value = 3 }; }

    static bool MakeControl()
    {
        var target = new Node { Value = 99 };
        Control = new WeakReference(target);
        bool alive = Control.IsAlive && Object.ReferenceEquals(Control.Target, target);
        GC.KeepAlive(target);
        return alive;
    }

    static void MakeLive()
    {
        Live = new Node { Value = 4 };
        while (!Stop) Thread.Sleep(1);
    }

    static bool CheckGraph()
    {
        Node? a = Root;
        Node? b = a?.Next;
        Node? c = b?.Next;
        Node? d = c?.Next;
        return a != null && b != null && c != null && d != null
            && a.Value == 1 && b.Value == 2 && c.Value == 3 && d.Value == 4
            && Object.ReferenceEquals(d.Next, a);
    }

    static int Main()
    {
        Root = new Node { Value = 1 };
        bool ok = MakeControl();
        var first = new Thread(MakeFirst);
        var second = new Thread(MakeSecond);
        var live = new Thread(MakeLive);
        first.Start(); first.Join();
        second.Start(); second.Join();
        live.Start();
        while (Live == null) Thread.Sleep(1);
        Root.Next = First;
        First!.Next = Second;
        Second!.Next = Live;
        Live!.Next = Root;
        First = null; Second = null; Live = null;
        for (int i = 0; i < 4; i++)
        {
            GC.Collect();
            GC.WaitForPendingFinalizers();
            ok = ok && CheckGraph();
        }
        bool reclaimed = Control != null && !Control.IsAlive && Control.Target == null;
        Stop = true;
        live.Join();
        GC.Collect();
        ok = ok && CheckGraph();
        Console.WriteLine(ok && reclaimed);
        return ok && reclaimed ? 42 : 1;
    }
}
