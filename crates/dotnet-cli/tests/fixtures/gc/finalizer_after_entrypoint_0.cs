using System;

class Payload { ~Payload() { } }
class Program { static void Main() { new Payload(); GC.Collect(); } }
