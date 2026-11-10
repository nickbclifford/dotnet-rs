using System;
using System.Threading;

public class Program {
    static Thread created;
    static int result;

    public static int Main() {
        Thread creator = new Thread(() => {
            int captured = 40;
            created = new Thread((object value) => { result = captured + (int)value; });
        });
        creator.Start();
        creator.Join();
        GC.Collect();
        created.Start(2);
        created.Join();
        return result;
    }
}
