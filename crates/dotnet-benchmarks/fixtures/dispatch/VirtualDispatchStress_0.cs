using System;

public interface IWorker {
    int Work(int input);
}

public class AddWorker : IWorker {
    public int Work(int input) => input + 1;
}

public class MulWorker : IWorker {
    public int Work(int input) => input * 3;
}

public class XorWorker : IWorker {
    public int Work(int input) => input ^ 0x5A5A;
}

public class Program {
    public static int Main() {
        IWorker[] workers = new IWorker[] {
            new AddWorker(),
            new MulWorker(),
            new XorWorker(),
            new AddWorker(),
            new MulWorker(),
        };

        int value = 7;
        for (int i = 0; i < 200_000; i++) {
            IWorker worker = workers[i % workers.Length];
            value = worker.Work(value);
            value &= 0x7FFF_FFFF;
        }

        return value != 0 ? 0 : 1;
    }
}
