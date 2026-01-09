using System;
using System.Threading;

public class Program {
    public static int Main() {
        var threads = new Thread[20];
        for (int i = 0; i < 20; i++) {
            var t = new Thread(() => {
                for (int j = 0; j < 1000; j++) {
                    var type1 = typeof(int);
                    var type2 = typeof(string);
                    var type3 = typeof(Thread);
                    
                    if (type1 == null || type2 == null || type3 == null) {
                        Environment.Exit(1);
                    }
                }
            });
            threads[i] = t;
            t.Start();
        }

        foreach (var t in threads) {
            t.Join();
        }

        return 0;
    }
}
