const os = require('os');

console.log("Hello from NodeJS Agent!");
console.log(`Node Version: ${process.version}`);
console.log(`Architecture: ${os.arch()} / Platform: ${os.platform()}`);

// Small calculation
let res = 0;
for (let i = 1; i <= 100; i++) {
    res += i;
}
console.log(`Sum 1 to 100 is: ${res}`);

process.exit(0);
