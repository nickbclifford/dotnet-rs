using System;

public delegate void CollectWith(int generation, GCCollectionMode mode);
public class Program {
    public static int Main() {
        Action none = GC.Collect;
        none += () => Console.WriteLine("after-0");
        none += GC.Collect;
        none();

        Action<int> generation = GC.Collect;
        generation += g => Console.WriteLine("after-1 " + g);
        generation += GC.Collect;
        generation(0);

        CollectWith mode = GC.Collect;
        mode += (g, m) => Console.WriteLine("after-2 " + g + " " + m);
        mode += GC.Collect;
        mode(0, GCCollectionMode.Forced);

        Console.WriteLine("done");
        return 42;
    }
}
