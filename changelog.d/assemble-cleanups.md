Changed

- Connect names the sigop-cost cap and subtracts fees with checked
  arithmetic. An output sum above `MAX_MONEY` is still rejected in
  structure validation (`bad-txns-txouttotal-toolarge`) before that sum
  is cast. Thanks to @Hero-Gamer.
